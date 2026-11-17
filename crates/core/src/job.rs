//! A serial conversion queue running on a background thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::convert::{self, ConvertOptions, OutputFormat, Plan};
use crate::ffmpeg::{Ffmpeg, RunOutcome};
use crate::sequence::{self, SequenceOptions};
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
    /// The probed input, or the pictures of an animation in the order given.
    pub inputs: Vec<MediaInfo>,
    /// The exact command(s) that will run.
    pub command: String,
}

/// What [`Queue::save_frame`] wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFrame {
    pub output: PathBuf,
    pub output_bytes: u64,
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
        if let Some(message) = ffmpeg.missing_output(options) {
            return Err(Error::Unsupported { message });
        }
        let info = ffmpeg.probe(input)?;
        if let Some(message) = ffmpeg.unreadable(&info) {
            return Err(Error::Unsupported { message });
        }
        if options.format.animates(info.kind)
            && let Some(message) = ffmpeg.missing_animation(options.format)
        {
            return Err(Error::Unsupported { message });
        }

        let plan = {
            let mut reserved = lock(&self.shared.reserved);
            let output = convert::output_path(&info.path, options.format, |p| reserved.contains(p));
            let plan = convert::plan(&info, options, &output, ffmpeg.support())?;
            reserved.insert(output);
            plan
        };
        self.enqueue(plan, vec![info])
    }

    /// Probe `pictures`, plan one animation from them in this order (see
    /// [`sequence::plan_sequence`]) and queue it.
    ///
    /// The output goes next to the first picture (see
    /// [`sequence::sequence_output_path`]). A `Queued` event is emitted before
    /// this returns.
    ///
    /// # Errors
    ///
    /// Probe errors of any picture, [`Error::Unsupported`] if the request
    /// doesn't make an animation or the tools to write it are missing.
    pub fn submit_sequence(
        &self,
        pictures: &[impl AsRef<Path>],
        options: &ConvertOptions,
        sequence: &SequenceOptions,
    ) -> Result<JobTicket> {
        let ffmpeg = &self.shared.ffmpeg;
        if let Some(message) = ffmpeg
            .missing_animation(options.format)
            .or_else(|| ffmpeg.missing_output(options))
        {
            return Err(Error::Unsupported { message });
        }
        let infos = pictures
            .iter()
            .map(|path| ffmpeg.probe(path))
            .collect::<Result<Vec<_>>>()?;
        if let Some(message) = infos.iter().find_map(|info| ffmpeg.unreadable(info)) {
            return Err(Error::Unsupported { message });
        }

        let plan = {
            let mut reserved = lock(&self.shared.reserved);
            let output =
                sequence::sequence_output_path(&infos, options.format, |p| reserved.contains(p));
            let plan =
                sequence::plan_sequence(&infos, options, sequence, &output, ffmpeg.support())?;
            reserved.insert(output);
            plan
        };
        self.enqueue(plan, infos)
    }

    /// Register a planned job whose output is already reserved and hand it to
    /// the worker.
    fn enqueue(&self, plan: Plan, inputs: Vec<MediaInfo>) -> Result<JobTicket> {
        let (id, cancel) = (JobId::next(), Arc::new(AtomicBool::new(false)));
        lock(&self.shared.active).insert(id, Arc::clone(&cancel));

        let ticket = JobTicket {
            id,
            output: plan.output.clone(),
            inputs,
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

    /// Save the frame of `input` at `at_secs` as a PNG next to it (see
    /// [`convert::frame_plan`]), right away rather than in turn: one frame
    /// takes a moment. Blocks until the file is written. The name is kept
    /// clear of outputs that queued jobs will write.
    ///
    /// # Errors
    ///
    /// Probe errors, [`Error::Unsupported`] if `input` is not a video or this
    /// ffmpeg can't write PNG, and [`Error::Ffmpeg`] if the grab fails.
    pub fn save_frame(
        &self,
        input: impl AsRef<Path>,
        options: &ConvertOptions,
        at_secs: f64,
    ) -> Result<SavedFrame> {
        let ffmpeg = &self.shared.ffmpeg;
        if let Some(message) = ffmpeg.missing(OutputFormat::Png) {
            return Err(Error::Unsupported { message });
        }
        let info = ffmpeg.probe(input)?;
        if let Some(message) = ffmpeg.unreadable(&info) {
            return Err(Error::Unsupported { message });
        }
        let plan = {
            let mut reserved = lock(&self.shared.reserved);
            let output = convert::frame_output_path(&info.path, at_secs, |p| reserved.contains(p));
            let plan = convert::frame_plan(&info, options, at_secs, &output, ffmpeg.support())?;
            reserved.insert(output);
            plan
        };
        let result = ffmpeg.run(&plan, &AtomicBool::new(false), |_| {});
        lock(&self.shared.reserved).remove(&plan.output);
        match result? {
            RunOutcome::Done { output_bytes } => Ok(SavedFrame {
                output: plan.output,
                output_bytes,
            }),
            RunOutcome::Cancelled => unreachable!("nothing raises the cancel flag"),
        }
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
    use crate::{OutputFormat, SequenceOptions};

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

        // A fast boomerang GIF: 1.2 s at 2× is 0.6 s, there and back
        // without repeating the turnaround frames: 9 + 7 frames at 15 fps.
        if ffmpeg.supports(OutputFormat::Gif) {
            let boomerang = ConvertOptions {
                format: OutputFormat::Gif,
                trim_start: Some(0.4),
                trim_end: Some(1.6),
                max_size: Some(60),
                speed: 2.0,
                direction: crate::Direction::Boomerang,
                ..ConvertOptions::default()
            };
            let ticket = queue.submit(&input, &boomerang).expect("submit");
            let done = finish_of(&rx, ticket.id);
            assert_eq!(done.stage, JobStage::Done, "{done:?}");
            let out = ffmpeg.probe(&ticket.output).expect("probe gif");
            let secs = out.duration_secs.expect("duration");
            assert!((secs - 16.0 / 15.0).abs() < 0.1, "{secs}");
        }

        // Preview helpers.
        let frame = ffmpeg.frame(&input, 1.0, 90, false).expect("frame");
        assert_eq!(&frame[..2], &[0xFF, 0xD8], "JPEG magic");
        let wave = ffmpeg
            .waveform(&input, 200, 40, "#c93b63")
            .expect("waveform");
        assert_eq!(&wave[1..4], b"PNG");
        assert!(ffmpeg.waveform(&input, 200, 40, "red;rm").is_err());

        // Saving a frame runs right away, with the turn and size but not the trim.
        if ffmpeg.supports(OutputFormat::Png) {
            let saved = queue.save_frame(&input, &options, 1.2).expect("save frame");
            assert_eq!(
                saved.output.file_name().and_then(|n| n.to_str()),
                Some("test clip-frame-0m01s.png")
            );
            assert!(saved.output_bytes > 0);
            let still = ffmpeg.probe(&saved.output).expect("probe frame");
            assert_eq!(still.kind, crate::MediaKind::Image);
            assert_eq!((still.width, still.height), (Some(120), Some(160)));
            let again = queue.save_frame(&input, &options, 1.4).expect("save again");
            assert!(again.output.ends_with("test clip-frame-0m01s 2.png"));
        }
    }

    /// A copy of the JPEG `src` with an EXIF orientation tag, as phones write
    /// portrait photos.
    fn with_orientation(src: &Path, dst: &Path, orientation: u16) {
        let jpeg = std::fs::read(src).expect("read jpeg");
        let exif = crate::media::tests::jpeg_with_orientation(orientation, true);
        // The test helper builds SOI, APP0, APP1, SOS; splice in its APP0+APP1.
        let sos = exif
            .windows(2)
            .rposition(|w| w == [0xFF, 0xDA])
            .expect("sos");
        let mut out = jpeg[..2].to_vec();
        out.extend(&exif[2..sos]);
        out.extend(&jpeg[2..]);
        std::fs::write(dst, out).expect("write jpeg");
    }

    /// Real ffmpeg runs of pictures → animation. Skipped (passes) when ffmpeg
    /// is not installed.
    #[test]
    fn pictures_become_animations_end_to_end() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping sequence test");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let make = |source: &str, name: &str| {
            let path = dir.path().join(name);
            let status = std::process::Command::new(ffmpeg.ffmpeg_path())
                .args(["-v", "error", "-f", "lavfi", "-i", source, "-frames:v", "1"])
                .arg(&path)
                .status()
                .expect("run ffmpeg");
            assert!(status.success(), "{name}");
            path
        };
        let a = make("testsrc2=s=640x480", "cat 1.jpg");
        let b = make("color=red:s=640x480,format=gray", "cat 2.jpg");
        let c = make("testsrc=s=300x500", "cat's 3.png");
        let landscape = make("testsrc2=s=400x300", "plain.jpg");
        let turned = dir.path().join("turned.jpg");
        with_orientation(&landscape, &turned, 6);

        let info = ffmpeg.probe(&turned).expect("probe");
        assert_eq!(info.orientation, Some(6));
        assert_eq!((info.width, info.height), (Some(300), Some(400)));

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });
        let run = |pictures: &[&Path], options: ConvertOptions, sequence: SequenceOptions| {
            let ticket = queue
                .submit_sequence(pictures, &options, &sequence)
                .expect("submit");
            assert_eq!(ticket.inputs.len(), pictures.len());
            let done = finish_of(&rx, ticket.id);
            assert_eq!(done.stage, JobStage::Done, "{done:?}\n{}", ticket.command);
            let out = ffmpeg.probe(&ticket.output).expect("probe output");
            (ticket, out)
        };

        // Mixed codecs, sizes and a grey picture; boomerang at 0.25 s.
        let quarter = SequenceOptions {
            frame_secs: 0.25,
            ..SequenceOptions::default()
        };
        let gif = ConvertOptions {
            format: OutputFormat::Gif,
            ..ConvertOptions::default()
        };
        let boomerang = ConvertOptions {
            direction: crate::Direction::Boomerang,
            ..gif.clone()
        };
        let (ticket, out) = run(&[&a, &b, &c, &turned], boomerang, quarter);
        // No shared name ("turned"), so it's named after the folder.
        let folder = dir.path().file_name().expect("name").to_string_lossy();
        let expected = format!("{}.gif", folder.trim_start_matches('.'));
        assert_eq!(ticket.output, dir.path().join(expected));
        assert_eq!(out.kind, crate::MediaKind::Video);
        assert_eq!((out.width, out.height), (Some(640), Some(480)));
        // a b c turned c b: six pictures of a quarter second.
        let secs = out.duration_secs.expect("duration");
        assert!((secs - 1.5).abs() < 0.02, "{secs}");

        // Portrait first: the frame is portrait, and an MP4 slideshow.
        let mp4 = ConvertOptions {
            format: OutputFormat::Mp4,
            ..ConvertOptions::default()
        };
        if ffmpeg.supports(OutputFormat::Mp4) {
            let crop = SequenceOptions {
                fit: crate::Fit::Crop,
                ..SequenceOptions::default()
            };
            let (_, out) = run(&[&turned, &a, &c], mp4, crop);
            assert_eq!((out.width, out.height), (Some(300), Some(400)));
            assert!((out.duration_secs.expect("duration") - 1.5).abs() < 0.05);
            assert!((out.fps.expect("fps") - 24.0).abs() < 0.1);
        }

        // Many sizes: each picture is scaled on its own first.
        let many: Vec<PathBuf> = (0..10)
            .map(|i| {
                make(
                    &format!("testsrc=s={}x{}", 100 + i * 10, 80),
                    &format!("p{i}.png"),
                )
            })
            .collect();
        let many: Vec<&Path> = many.iter().map(PathBuf::as_path).collect();
        let (ticket, out) = run(
            &many,
            gif.clone(),
            SequenceOptions {
                frame_secs: 0.1,
                ..SequenceOptions::default()
            },
        );
        assert!(
            ticket.command.matches("ffmpeg ").count() > 10,
            "{}",
            ticket.command
        );
        assert_eq!((out.width, out.height), (Some(100), Some(80)));
        assert!((out.duration_secs.expect("duration") - 1.0).abs() < 0.02);

        // Animated JPEG XL through cjxl, when installed.
        if ffmpeg.jxl_tools().cjxl.is_some() {
            let jxl = ConvertOptions {
                format: OutputFormat::Jxl,
                ..ConvertOptions::default()
            };
            let (ticket, _) = run(&[&a, &b], jxl, SequenceOptions::default());
            assert!(ticket.command.contains("cjxl "), "{}", ticket.command);
        }

        // A picture that has gone missing fails the submit, not the queue.
        let gone = dir.path().join("gone.jpg");
        assert!(
            queue
                .submit_sequence(&[&a, &gone], &gif, &SequenceOptions::default())
                .is_err()
        );

        // Only the pictures and outputs are left: no lists or scratch files.
        let hidden: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(hidden.is_empty(), "{hidden:?}");
    }

    /// AV1 out and back in: an HDR clip becomes 10-bit AV1 that stays HDR,
    /// and that file can be read, previewed and converted again. Skipped
    /// (passes) without an ffmpeg that has SVT-AV1 and a software AV1 decoder.
    #[test]
    fn av1_round_trip_end_to_end() {
        use crate::VideoCodec;

        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping AV1 end-to-end test");
            return;
        };
        if ffmpeg.missing_codec(VideoCodec::Av1).is_some() || !ffmpeg.reads_av1() {
            eprintln!("ffmpeg lacks SVT-AV1 or dav1d; skipping AV1 end-to-end test");
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        // 10-bit, tagged as HDR10 (PQ), with sound.
        let input = dir.path().join("hdr.mkv");
        let status = std::process::Command::new(ffmpeg.ffmpeg_path())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=320x240:d=1:r=25",
            ])
            .args(["-f", "lavfi", "-i", "sine=d=1", "-shortest"])
            .args([
                "-vf",
                "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
            ])
            .args([
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "yuv420p10le",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&input)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        assert!(ffmpeg.probe(&input).expect("probe").hdr);

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });
        let av1 = ConvertOptions {
            format: OutputFormat::Mp4,
            video_codec: Some(VideoCodec::Av1),
            max_size: Some(120),
            ..ConvertOptions::default()
        };
        let ticket = queue.submit(&input, &av1).expect("submit");
        let done = finish_of(&rx, ticket.id);
        assert_eq!(done.stage, JobStage::Done, "{done:?}");
        let out = ffmpeg.probe(&ticket.output).expect("probe AV1");
        assert_eq!(out.video_codec.as_deref(), Some("av1"));
        assert_eq!((out.width, out.height), (Some(160), Some(120)));
        assert!(out.hdr, "AV1 keeps the HDR transfer");
        assert!(out.has_audio());

        // Reading it back: a preview frame and another conversion.
        let frame = ffmpeg.frame(&ticket.output, 0.5, 60, true).expect("frame");
        assert_eq!(&frame[..2], &[0xFF, 0xD8], "JPEG magic");
        let webm = ConvertOptions {
            format: OutputFormat::Webm,
            ..ConvertOptions::default()
        };
        if ffmpeg.supports(OutputFormat::Webm) {
            let again = queue.submit(&ticket.output, &webm).expect("submit again");
            let done = finish_of(&rx, again.id);
            assert_eq!(done.stage, JobStage::Done, "{done:?}");
            let out = ffmpeg.probe(&again.output).expect("probe VP9");
            assert_eq!(out.video_codec.as_deref(), Some("vp9"));
        }

        // AV1 can't go into MOV.
        let mov = ConvertOptions {
            format: OutputFormat::Mov,
            ..av1
        };
        assert!(queue.submit(&input, &mov).is_err());
    }

    /// An exact bitrate: H.264 in two passes lands near the target, the
    /// first pass's log files are cleaned up, and the probe reads the
    /// bitrates back. Skipped (passes) without ffmpeg.
    #[test]
    fn exact_bitrate_end_to_end() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping exact-bitrate test");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("noise.mkv");
        let status = std::process::Command::new(ffmpeg.ffmpeg_path())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=320x240:d=3:r=25",
            ])
            .args(["-f", "lavfi", "-i", "sine=d=3", "-shortest"])
            .args(["-c:v", "ffv1", "-c:a", "pcm_s16le", "-ac", "2"])
            .arg(&input)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        let probed = ffmpeg.probe(&input).expect("probe");
        assert_eq!(probed.audio_channels, Some(2));
        assert!(probed.bitrate.is_some());

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });
        let options = ConvertOptions {
            video_bitrate: Some(400),
            ..ConvertOptions::default()
        };
        let ticket = queue.submit(&input, &options).expect("submit");
        assert!(ticket.command.contains("-pass 2"), "{}", ticket.command);
        let done = finish_of(&rx, ticket.id);
        assert_eq!(done.stage, JobStage::Done, "{done:?}");

        let out = ffmpeg.probe(&ticket.output).expect("probe output");
        let kbps = out.video_bitrate.expect("MP4 records the video bitrate") / 1000;
        assert!(
            (200..=600).contains(&kbps),
            "aimed for 400 kbps, got {kbps}"
        );
        assert_eq!(
            out.audio_bitrate.map(|b| b / 16_000),
            Some(128 / 16),
            "{out:?}"
        );
        let mut left: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["noise.mkv", "noise.mp4"], "pass logs are cleaned up");
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
    /// A geotagged photo through every path that has to drop its location.
    /// Skipped (passes) without ffmpeg and libjxl's tools.
    #[test]
    fn location_is_dropped_end_to_end() {
        use crate::scrub::tests::{mentions_make, with_exif};
        use crate::{Metadata, scrub::has_location};

        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping location test");
            return;
        };
        let tools = ffmpeg.jxl_tools();
        if tools.cjxl.is_none() || tools.djxl.is_none() {
            eprintln!("libjxl tools not found; skipping location test");
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let plain = dir.path().join("plain.jpg");
        let status = std::process::Command::new(ffmpeg.ffmpeg_path())
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=320x240:d=1"])
            .args(["-frames:v", "1", "-q:v", "3"])
            .arg(&plain)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        let photo = dir.path().join("photo.jpg");
        let original = with_exif(&std::fs::read(&plain).expect("read"), true);
        std::fs::write(&photo, &original).expect("write");
        assert!(ffmpeg.probe(&photo).expect("probe").has_location);

        let (tx, rx) = mpsc::channel();
        let queue = Queue::new(ffmpeg.clone(), move |event| {
            let _ = tx.send(event);
        });
        let run = |options: ConvertOptions| {
            let ticket = queue.submit(&photo, &options).expect("submit");
            assert_eq!(
                finish_of(&rx, ticket.id).stage,
                JobStage::Done,
                "{}",
                ticket.command
            );
            ticket
        };

        // Lossless JPEG XL stays a repack, and restores to the photo minus its GPS.
        let jxl = run(ConvertOptions {
            format: OutputFormat::Jxl,
            jxl: crate::JxlOptions {
                lossless: true,
                ..crate::JxlOptions::default()
            },
            ..ConvertOptions::default()
        });
        assert!(
            jxl.command.starts_with("catconvert scrub "),
            "{}",
            jxl.command
        );
        let packed = ffmpeg.probe(&jxl.output).expect("probe jxl");
        assert!(packed.reconstructs_jpeg);
        assert!(!packed.has_location);
        let restored = dir.path().join("restored.jpg");
        let djxl = std::process::Command::new(tools.djxl.as_ref().expect("djxl"))
            .arg(&jxl.output)
            .arg(&restored)
            .args(["--reconstruct_jpeg", "--quiet"])
            .status()
            .expect("run djxl");
        assert!(djxl.success());
        let expected = dir.path().join("expected.jpg");
        crate::scrub_file(&photo, &expected, Metadata::NoLocation).expect("scrub");
        let restored = std::fs::read(&restored).expect("read");
        assert_eq!(restored, std::fs::read(&expected).expect("read"));
        assert_eq!(restored.len(), original.len());
        assert!(mentions_make(&restored));

        // ffmpeg writes the EXIF into PNGs; it is cleaned afterwards.
        let png = |metadata| {
            let ticket = run(ConvertOptions {
                format: OutputFormat::Png,
                metadata,
                ..ConvertOptions::default()
            });
            (
                has_location(&ticket.output),
                mentions_make(&std::fs::read(&ticket.output).expect("read")),
            )
        };
        assert_eq!(png(Metadata::Keep), (true, true));
        assert_eq!(png(Metadata::NoLocation), (false, true));
        assert_eq!(png(Metadata::None), (false, false));
        assert!(
            std::fs::read_dir(dir.path()).expect("list").all(|entry| {
                !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with('.')
            }),
            "scratch files are cleaned up"
        );
    }

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

        // Turned and cropped square.
        let square = ConvertOptions {
            crop: Some(crate::Crop {
                aspect: Some([1, 1]),
                area: None,
            }),
            ..turned
        };
        let ticket = queue.submit(&jpeg, &square).expect("submit");
        assert_eq!(finish_of(&rx, ticket.id).stage, JobStage::Done);
        let out = ffmpeg.probe(&ticket.output).expect("probe");
        assert_eq!((out.width, out.height), (Some(240), Some(240)));

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
        let frame = ffmpeg.frame(&ticket.output, 0.8, 90, false).expect("frame");
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

        let frame = ffmpeg.frame(&heic, 0.0, 90, false).expect("frame");
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
