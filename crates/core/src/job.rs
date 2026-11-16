use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

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

/// A conversion request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: JobId,
    pub input: PathBuf,
    /// Target format as a file extension, e.g. `"webp"` or `"mp3"`.
    pub target_format: String,
}

impl Job {
    /// Create a job with a fresh [`JobId`].
    #[must_use]
    pub fn new(input: impl Into<PathBuf>, target_format: impl Into<String>) -> Self {
        Self {
            id: JobId::next(),
            input: input.into(),
            target_format: target_format.into(),
        }
    }
}

/// Lifecycle stage reported with each progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStage {
    Queued,
    Running,
    Done,
}

/// Progress event sent while a job runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: JobId,
    pub stage: JobStage,
    /// Completion between `0.0` and `1.0`.
    pub progress: f32,
}

/// Timing of the fake progress stream emitted by the stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StubTiming {
    /// Number of `Running` events between `Queued` and `Done`.
    pub steps: u32,
    /// Pause between events.
    pub delay: Duration,
}

impl Default for StubTiming {
    fn default() -> Self {
        Self {
            steps: 20,
            delay: Duration::from_millis(100),
        }
    }
}

/// Handle to a job running on a background thread.
#[derive(Debug)]
pub struct JobHandle {
    pub id: JobId,
    thread: JoinHandle<()>,
}

impl JobHandle {
    /// Block until the job has finished emitting events.
    pub fn join(self) {
        // The stub never panics; if a future implementation does, surface it.
        if let Err(panic) = self.thread.join() {
            std::panic::resume_unwind(panic);
        }
    }
}

/// Start a job with default [`StubTiming`].
///
/// Stub: does not touch the input file. It sends `Queued`, a series of
/// `Running` events with increasing progress, then `Done` over `events`.
/// If the receiver is dropped, the job stops early.
#[must_use = "join the handle or let it run detached"]
pub fn submit_job(job: Job, events: Sender<JobProgress>) -> JobHandle {
    submit_job_with(job, events, StubTiming::default())
}

/// Like [`submit_job`] with explicit timing (useful for tests and demos).
#[must_use = "join the handle or let it run detached"]
pub fn submit_job_with(job: Job, events: Sender<JobProgress>, timing: StubTiming) -> JobHandle {
    let id = job.id;
    let thread = std::thread::Builder::new()
        .name(format!("catconvert-job-{id}"))
        .spawn(move || run_stub(id, &events, timing))
        .expect("failed to spawn job thread");
    JobHandle { id, thread }
}

fn run_stub(id: JobId, events: &Sender<JobProgress>, timing: StubTiming) {
    let send = |stage, progress| {
        events
            .send(JobProgress {
                job_id: id,
                stage,
                progress,
            })
            .is_ok()
    };

    if !send(JobStage::Queued, 0.0) {
        return;
    }
    let steps = timing.steps.max(1);
    for step in 1..=steps {
        std::thread::sleep(timing.delay);
        #[allow(clippy::cast_precision_loss)]
        let progress = step as f32 / steps as f32;
        if !send(JobStage::Running, progress) {
            return;
        }
    }
    send(JobStage::Done, 1.0);
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    const FAST: StubTiming = StubTiming {
        steps: 4,
        delay: Duration::ZERO,
    };

    #[test]
    fn job_ids_are_unique() {
        let a = Job::new("a.png", "webp");
        let b = Job::new("b.png", "webp");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn stub_emits_queued_running_done() {
        let (tx, rx) = mpsc::channel();
        let job = Job::new("in.wav", "mp3");
        let id = job.id;
        let handle = submit_job_with(job, tx, FAST);
        assert_eq!(handle.id, id);
        handle.join();

        let events: Vec<_> = rx.iter().collect();
        assert_eq!(events.len(), 6, "queued + 4 running + done: {events:?}");
        assert!(events.iter().all(|e| e.job_id == id));
        assert_eq!(events.first().map(|e| e.stage), Some(JobStage::Queued));
        assert_eq!(events.last().map(|e| e.stage), Some(JobStage::Done));
        assert!(
            events.windows(2).all(|w| w[0].progress <= w[1].progress),
            "progress must be monotonic: {events:?}"
        );
        assert!((events[4].progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn stub_stops_when_receiver_is_dropped() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        // Must return instead of looping or panicking.
        submit_job_with(Job::new("x.mov", "mp4"), tx, FAST).join();
    }

    #[test]
    fn progress_serializes_camel_case() {
        let event = JobProgress {
            job_id: JobId(7),
            stage: JobStage::Running,
            progress: 0.5,
        };
        let json = serde_json::to_value(event).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({"jobId": 7, "stage": "running", "progress": 0.5})
        );
    }
}
