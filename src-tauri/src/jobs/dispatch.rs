//! Delivers job updates to the sink from one thread, in batches.
//!
//! The queue hands each update over while holding its lock, so they arrive
//! here in `seq` order. This one thread calls the sink, outside the queue's
//! lock, so they reach the frontend in that same order however many
//! workers there are. Updates that arrive within [`BATCH_WINDOW`] of each
//! other go out together, keeping only the newest per job, so thousands of
//! quick changes cost a handful of events.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use super::events::JobUpdate;

/// How long a batch collects updates after its first one.
pub const BATCH_WINDOW: Duration = Duration::from_millis(50);

/// Where batches of job updates go: to the webview in the app, to a list
/// in tests. Only the dispatch thread calls it.
pub type EventSink = Box<dyn Fn(&[JobUpdate]) + Send>;

/// Starts the dispatch thread. It stops once every sender is dropped,
/// after sending what it holds.
pub(super) fn start(sink: EventSink) -> std::io::Result<mpsc::Sender<JobUpdate>> {
    let (outbox, updates) = mpsc::channel();
    thread::Builder::new()
        .name("job-updates".into())
        .spawn(move || run(&updates, &sink))?;
    Ok(outbox)
}

fn run(updates: &mpsc::Receiver<JobUpdate>, sink: &EventSink) {
    while let Ok(first) = updates.recv() {
        let mut batch = vec![first];
        let deadline = Instant::now() + BATCH_WINDOW;
        let open = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match updates.recv_timeout(left) {
                Ok(update) => batch.push(update),
                Err(RecvTimeoutError::Timeout) => break true,
                Err(RecvTimeoutError::Disconnected) => break false,
            }
        };
        sink(&coalesce(batch));
        if !open {
            return;
        }
    }
}

/// Keeps only the newest update for each job, in `seq` order. Each update
/// holds the job's whole state, so the newest one says everything.
pub fn coalesce(batch: Vec<JobUpdate>) -> Vec<JobUpdate> {
    let mut newest: HashMap<_, JobUpdate> = HashMap::with_capacity(batch.len());
    for update in batch {
        match newest.get(&update.id) {
            Some(kept) if kept.seq > update.seq => {}
            _ => {
                newest.insert(update.id, update);
            }
        }
    }
    let mut kept: Vec<_> = newest.into_values().collect();
    kept.sort_by_key(|u| u.seq);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobId, JobKind, JobStatus};
    use std::sync::{Arc, Mutex};

    fn update(seq: u64, id: i64, status: JobStatus) -> JobUpdate {
        JobUpdate {
            seq,
            id: JobId(id),
            kind: JobKind::Hash,
            status,
            progress: None,
            priority: 0,
        }
    }

    #[test]
    fn coalescing_keeps_the_newest_update_per_job_in_seq_order() {
        use JobStatus::*;
        let batch = vec![
            update(1, 1, Queued),
            update(2, 2, Queued),
            update(3, 1, Running),
            update(4, 3, Queued),
            update(5, 2, Cancelled),
            update(6, 1, Done),
        ];
        let kept: Vec<_> = coalesce(batch)
            .into_iter()
            .map(|u| (u.seq, u.id.0, u.status))
            .collect();
        assert_eq!(kept, [(4, 3, Queued), (5, 2, Cancelled), (6, 1, Done)]);
    }

    #[test]
    fn a_burst_of_updates_goes_out_in_a_few_batches_in_order() {
        let batches = Arc::new(Mutex::new(Vec::<Vec<JobUpdate>>::new()));
        let seen = batches.clone();
        let outbox = start(Box::new(move |b: &[JobUpdate]| {
            seen.lock().unwrap().push(b.to_vec())
        }))
        .unwrap();
        for seq in 1..=10_000u64 {
            let id = (seq % 500) as i64;
            outbox.send(update(seq, id, JobStatus::Running)).unwrap();
        }
        drop(outbox);
        let start = Instant::now();
        let flushed = || {
            batches
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .any(|u| u.seq == 10_000)
        };
        while !flushed() {
            assert!(start.elapsed() < Duration::from_secs(10), "never flushed");
            thread::sleep(Duration::from_millis(5));
        }
        let batches = batches.lock().unwrap();
        assert!(batches.len() <= 5, "{} batches", batches.len());
        let seqs: Vec<_> = batches.iter().flatten().map(|u| u.seq).collect();
        assert!(seqs.windows(2).all(|w| w[1] > w[0]), "out of order");
        // The last batch holds each job's newest state.
        assert_eq!(*seqs.last().unwrap(), 10_000);
    }
}
