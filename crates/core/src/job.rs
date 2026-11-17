//! A serial conversion queue running on a background thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::convert::{self, ConvertOptions, Plan};
use crate::ffmpeg::{Ffmpeg, RunOutcome};
use crate::{Error, MediaInfo, Result};

/// Process-unique identifier of a conversion job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl JobId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Lifecycle stage reported with each progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStage {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStage {
    /// Whether the job will send no further events.
    #[must_use]
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// Event sent whenever a job changes stage or makes progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: JobId,
    pub stage: JobStage,
    /// Completion between `0.0` and `1.0`.
    pub progress: f32,
    /// Encoding speed relative to real time while running.
    pub speed: Option<f32>,
    /// Size of the finished file (`Done` only).
    pub output_bytes: Option<u64>,
    /// What went wrong (`Failed` only).
    pub error: Option<String>,
}

impl JobProgress {
    fn new(job_id: JobId, stage: JobStage, progress: f32) -> Self {
        Self {
            job_id,
            stage,
            progress,
            speed: None,
            output_bytes: None,
            error: None,
        }
    }
}

/// What [`Queue::submit`] hands back for an accepted job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobTicket {
    pub id: JobId,
    /// Where the result will be written.
    pub output: PathBuf,
    /// The probed input.
    pub input: MediaInfo,
    /// The exact command(s) that will run.
    pub command: String,
}

type Emit = dyn Fn(JobProgress) + Send + Sync;

struct Job {
    id: JobId,
    plan: Plan,
    cancel: Arc<AtomicBool>,
}

struct Shared {
    ffmpeg: Ffmpeg,
    emit: Box<Emit>,
    /// Cancel flags of jobs that are queued or running.
    active: Mutex<HashMap<JobId, Arc<AtomicBool>>>,
    /// Output paths promised to queued jobs but not on disk yet.
    reserved: Mutex<HashSet<PathBuf>>,
}

/// Runs conversions one at a time, in submission order, on a worker thread.
///
/// Every state change is reported through the `emit` callback given to
/// [`Queue::new`]. Dropping the queue lets the worker finish what is queued
/// and then exit.
pub struct Queue {
    shared: Arc<Shared>,
    tx: Sender<Job>,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue")
            .field("ffmpeg", &self.shared.ffmpeg)
            .finish_non_exhaustive()
    }
}

impl Queue {
    /// Start the worker thread.
    pub fn new(ffmpeg: Ffmpeg, emit: impl Fn(JobProgress) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            ffmpeg,
            emit: Box::new(emit),
            active: Mutex::default(),
            reserved: Mutex::default(),
        });
        let (tx, rx) = mpsc::channel::<Job>();
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("catconvert-queue".into())
            .spawn(move || {
                for job in rx {
                    worker.run(job);
                }
            })
            .expect("failed to spawn queue thread");
        Self { shared, tx }
    }

    /// The ffmpeg this queue runs.
    #[must_use]
    pub fn ffmpeg(&self) -> &Ffmpeg {
        &self.shared.ffmpeg
    }

    /// Probe `input`, plan the conversion and queue it.
    ///
    /// The output goes next to the input under a name that is not taken (see
    /// [`convert::output_path`]). A `Queued` event is emitted before this returns.
    ///
    /// # Errors
    ///
    /// Probe errors, [`Error::Unsupported`] if the options do not fit the input
    /// or the tools to write the format are missing.
    pub fn submit(&self, input: impl AsRef<Path>, options: &ConvertOptions) -> Result<JobTicket> {
        let ffmpeg = &self.shared.ffmpeg;
        if let Some(message) = ffmpeg.missing(options.format) {
            return Err(Error::Unsupported { message });
        }
        let info = ffmpeg.probe(input)?;

        let (plan, id, cancel) = {
            let mut reserved = lock(&self.shared.reserved);
            let output = convert::output_path(&info.path, options.format, |p| reserved.contains(p));
            let plan = convert::plan(&info, options, &output, ffmpeg.jxl_support())?;
            reserved.insert(output);
            (plan, JobId::next(), Arc::new(AtomicBool::new(false)))
        };
        lock(&self.shared.active).insert(id, Arc::clone(&cancel));

        let ticket = JobTicket {
            id,
            output: plan.output.clone(),
            input: info,
            command: plan.command_line(),
        };
        (self.shared.emit)(JobProgress::new(id, JobStage::Queued, 0.0));
        if self.tx.send(Job { id, plan, cancel }).is_err() {
            // The worker only stops when the queue is dropped, so this is unreachable
            // in practice; undo the bookkeeping anyway.
            self.shared.finish(id, &ticket.output);
            return Err(Error::Ffmpeg {
                message: "the conversion queue has stopped".into(),
            });
        }
        Ok(ticket)
    }

    /// Cancel a queued or running job. Returns `false` if it already finished.
    pub fn cancel(&self, id: JobId) -> bool {
        lock(&self.shared.active).get(&id).is_some_and(|flag| {
            flag.store(true, Ordering::Relaxed);
            true
        })
    }
}

impl Shared {
    fn run(&self, job: Job) {
        let Job { id, plan, cancel } = job;
        let emit = &self.emit;

        if cancel.load(Ordering::Relaxed) {
            emit(JobProgress::new(id, JobStage::Cancelled, 0.0));
            self.finish(id, &plan.output);
            return;
        }

        emit(JobProgress::new(id, JobStage::Running, 0.0));
        let mut last = 0.0_f32;
        let result = self.ffmpeg.run(&plan, &cancel, |sample| {
            last = sample.fraction.unwrap_or(last).max(last);
            emit(JobProgress {
                speed: sample.speed,
                ..JobProgress::new(id, JobStage::Running, last)
            });
        });

        let event = match result {
            Ok(RunOutcome::Done { output_bytes }) => JobProgress {
                output_bytes: Some(output_bytes),
                ..JobProgress::new(id, JobStage::Done, 1.0)
            },
            Ok(RunOutcome::Cancelled) => JobProgress::new(id, JobStage::Cancelled, last),
            Err(err) => JobProgress {
                error: Some(err.to_string()),
                ..JobProgress::new(id, JobStage::Failed, last)
            },
        };
        // Release bookkeeping first so a listener reacting to the final event
        // (e.g. retrying) sees a consistent queue.
        self.finish(id, &plan.output);
        emit(event);
    }

    fn finish(&self, id: JobId, output: &Path) {
        lock(&self.active).remove(&id);
        lock(&self.reserved).remove(output);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::OutputFormat;

    #[test]
    fn job_ids_are_unique() {
        assert_ne!(JobId::next(), JobId::next());
    }

    #[test]
    fn progress_serializes_camel_case() {
        let event = JobProgress {
            speed: Some(2.0),
            ..JobProgress::new(JobId(7), JobStage::Running, 0.5)
        };
        let json = serde_json::to_value(event).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "jobId": 7, "stage": "running", "progress": 0.5,
                "speed": 2.0, "outputBytes": null, "error": null
            })
        );
    }

    /// Real ffmpeg round trip. Skipped (passes) when ffmpeg is not installed.
    #[test]
    fn converts_a_generated_clip_end_to_end() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping end-to-end test");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("test clip.mov");
        let status = std::process::Command::new(ffmpeg.ffmpeg_path())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=320x240:d=2:r=25",
            ])
            .args(["-f", "lavfi", "-i", "sine=d=2", "-shortest"])
            .args(["-c:v", "mjpeg", "-c:a", "pcm_s16le"])
            .arg(&input)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());

        let info = ffmpeg.probe(&input).expect("probe");
        assert_eq!(info.kind, crate::MediaKind::Video);
        assert_eq!((info.width, info.height), (Some(320), Some(240)));
        assert!(info.has_audio());

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });

        let format = [OutputFormat::Mp4, OutputFormat::Gif, OutputFormat::Png]
            .into_iter()
            .find(|f| ffmpeg.supports(*f))
            .expect("some encoder");
        let options = ConvertOptions {
            format,
            trim_start: Some(0.5),
            trim_end: Some(1.5),
            max_size: Some(120),
            rotate: 90,
            ..ConvertOptions::default()
        };
        let ticket = queue.submit(&input, &options).expect("submit");
        assert_eq!(
            ticket.output.extension().and_then(|e| e.to_str()),
            Some(format.extension())
        );

        let mut stages = Vec::new();
        let done = loop {
            let event = rx.recv_timeout(Duration::from_secs(60)).expect("event");
            assert_eq!(event.job_id, ticket.id);
            stages.push(event.stage);
            if event.stage.is_final() {
                break event;
            }
        };
        assert_eq!(done.stage, JobStage::Done, "{done:?}");
        assert_eq!(stages.first(), Some(&JobStage::Queued));
        assert!(stages.contains(&JobStage::Running));
        assert!(done.output_bytes.unwrap_or(0) > 0);

        let out = ffmpeg.probe(&ticket.output).expect("probe output");
        // 320x240 rotated 90° is 240x320; short side capped at 120.
        assert_eq!(out.width, Some(120));
        assert_eq!(out.height, Some(160));

        // A second identical job must not collide with the first output.
        let again = queue.submit(&input, &options).expect("submit again");
        assert_ne!(again.output, ticket.output);
        queue.cancel(again.id);
        let last = loop {
            let event = rx.recv_timeout(Duration::from_secs(60)).expect("event");
            if event.job_id == again.id && event.stage.is_final() {
                break event;
            }
        };
        match last.stage {
            JobStage::Cancelled => assert!(!again.output.exists(), "partial output removed"),
            stage => assert_eq!(stage, JobStage::Done),
        }

        // Preview helpers.
        let frame = ffmpeg.frame(&input, 1.0, 90).expect("frame");
        assert_eq!(&frame[..2], &[0xFF, 0xD8], "JPEG magic");
        let wave = ffmpeg
            .waveform(&input, 200, 40, "#c93b63")
            .expect("waveform");
        assert_eq!(&wave[1..4], b"PNG");
        assert!(ffmpeg.waveform(&input, 200, 40, "red;rm").is_err());
    }

    /// Wait for the final event of `id`.
    fn finish_of(rx: &mpsc::Receiver<JobProgress>, id: JobId) -> JobProgress {
        loop {
            let event = rx.recv_timeout(Duration::from_secs(120)).expect("event");
            if event.job_id == id && event.stage.is_final() {
                return event;
            }
        }
    }

    /// Real JPEG XL round trips through cjxl/djxl. Skipped (passes) when
    /// ffmpeg or the libjxl tools are not installed.
    #[test]
    fn jpeg_xl_end_to_end() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping JPEG XL test");
            return;
        };
        let tools = ffmpeg.jxl_tools();
        if tools.cjxl.is_none() || tools.djxl.is_none() || tools.jxlinfo.is_none() {
            eprintln!("libjxl tools not found; skipping JPEG XL test");
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let make = |args: &[&str], out: &Path| {
            let status = std::process::Command::new(ffmpeg.ffmpeg_path())
                .args(["-v", "error", "-f", "lavfi", "-i"])
                .args(args)
                .arg(out)
                .status()
                .expect("run ffmpeg");
            assert!(status.success());
        };
        let jpeg = dir.path().join("photo.jpg");
        make(
            &["testsrc2=s=320x240:d=1", "-frames:v", "1", "-q:v", "3"],
            &jpeg,
        );
        let clip = dir.path().join("clip.mov");
        make(&["testsrc2=s=320x240:d=2:r=10", "-c:v", "mjpeg"], &clip);

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });
        let lossless = ConvertOptions {
            format: OutputFormat::Jxl,
            jxl: crate::JxlOptions {
                lossless: true,
                ..crate::JxlOptions::default()
            },
            ..ConvertOptions::default()
        };

        // JPEG → JXL losslessly, then back to the identical JPEG.
        let ticket = queue.submit(&jpeg, &lossless).expect("submit");
        assert!(ticket.command.starts_with("cjxl "), "{}", ticket.command);
        assert_eq!(finish_of(&rx, ticket.id).stage, JobStage::Done);
        let jxl = ffmpeg.probe(&ticket.output).expect("probe jxl");
        assert_eq!(jxl.kind, crate::MediaKind::Image);
        assert_eq!((jxl.width, jxl.height), (Some(320), Some(240)));
        assert!(jxl.reconstructs_jpeg);
        assert!(jxl.size_bytes < std::fs::metadata(&jpeg).expect("meta").len());

        let back = queue
            .submit(
                &ticket.output,
                &ConvertOptions {
                    format: OutputFormat::Jpg,
                    ..ConvertOptions::default()
                },
            )
            .expect("submit");
        assert!(
            back.command.contains("--reconstruct_jpeg"),
            "{}",
            back.command
        );
        assert_eq!(finish_of(&rx, back.id).stage, JobStage::Done);
        assert_eq!(
            std::fs::read(&back.output).expect("read"),
            std::fs::read(&jpeg).expect("read"),
            "restored JPEG is bit-identical"
        );

        // Edited still: ffmpeg → PNG → cjxl, scratch file cleaned up.
        let turned = ConvertOptions {
            format: OutputFormat::Jxl,
            rotate: 90,
            ..ConvertOptions::default()
        };
        let ticket = queue.submit(&jpeg, &turned).expect("submit");
        assert_eq!(finish_of(&rx, ticket.id).stage, JobStage::Done);
        let out = ffmpeg.probe(&ticket.output).expect("probe");
        assert_eq!((out.width, out.height), (Some(240), Some(320)));

        // Video → animated JXL.
        let animated = ConvertOptions {
            format: OutputFormat::Jxl,
            trim_start: Some(0.5),
            max_size: Some(120),
            ..ConvertOptions::default()
        };
        let ticket = queue.submit(&clip, &animated).expect("submit");
        let done = finish_of(&rx, ticket.id);
        assert_eq!(done.stage, JobStage::Done, "{done:?}");
        let anim = ffmpeg.probe(&ticket.output).expect("probe animation");
        assert_eq!(anim.kind, crate::MediaKind::Video);
        assert_eq!((anim.width, anim.height), (Some(160), Some(120)));
        let secs = anim.duration_secs.expect("duration");
        assert!((secs - 1.5).abs() < 0.15, "{secs}");
        assert!((anim.fps.expect("fps") - 10.0).abs() < 0.5);

        // Animated JXL → trimmed GIF via djxl, and previews of it.
        let gif = ConvertOptions {
            format: OutputFormat::Gif,
            trim_end: Some(1.0),
            ..ConvertOptions::default()
        };
        let ticket2 = queue.submit(&ticket.output, &gif).expect("submit");
        assert_eq!(finish_of(&rx, ticket2.id).stage, JobStage::Done);
        let back = ffmpeg.probe(&ticket2.output).expect("probe gif");
        assert_eq!(back.kind, crate::MediaKind::Video);
        assert!((back.duration_secs.expect("duration") - 1.0).abs() < 0.15);
        let frame = ffmpeg.frame(&ticket.output, 0.8, 90).expect("frame");
        assert_eq!(&frame[..2], &[0xFF, 0xD8], "JPEG magic");

        // No scratch files left behind.
        let hidden: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(hidden.is_empty(), "{hidden:?}");
    }

    /// A tiled HEIC like an iPhone's: probe, preview and convert the whole
    /// picture, not its first tile. ffmpeg can't write HEIC, so this needs
    /// macOS's `sips` to make one; skipped (passes) without it, and with an
    /// ffprobe older than 7 that knows no tile grids.
    #[test]
    fn tiled_heic_end_to_end() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping HEIC test");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let jpeg = dir.path().join("photo.jpg");
        let status = std::process::Command::new(ffmpeg.ffmpeg_path())
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=1200x900"])
            .args(["-frames:v", "1"])
            .arg(&jpeg)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        let heic = dir.path().join("IMG_0001.HEIC");
        let made = std::process::Command::new("sips")
            .args(["-s", "format", "heic"])
            .arg(&jpeg)
            .arg("--out")
            .arg(&heic)
            .output();
        if !made.is_ok_and(|out| out.status.success()) {
            eprintln!("sips not available; skipping HEIC test");
            return;
        }

        let info = ffmpeg.probe(&heic).expect("probe");
        assert_eq!(info.kind, crate::MediaKind::Image);
        if info.tile_grid.is_none() {
            eprintln!("this ffprobe knows no tile grids; skipping HEIC test");
            return;
        }
        assert_eq!((info.width, info.height), (Some(1200), Some(900)));

        let frame = ffmpeg.frame(&heic, 0.0, 90).expect("frame");
        assert_eq!(&frame[..2], &[0xFF, 0xD8], "JPEG magic");

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });
        for (max_size, size) in [(None, (1200, 900)), (Some(300), (400, 300))] {
            let options = ConvertOptions {
                format: OutputFormat::Png,
                max_size,
                ..ConvertOptions::default()
            };
            let ticket = queue.submit(&heic, &options).expect("submit");
            assert_eq!(finish_of(&rx, ticket.id).stage, JobStage::Done);
            let out = ffmpeg.probe(&ticket.output).expect("probe png");
            assert_eq!((out.width, out.height), (Some(size.0), Some(size.1)));
            std::fs::remove_file(&ticket.output).expect("remove output");
        }
    }
}
