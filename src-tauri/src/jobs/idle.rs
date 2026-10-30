//! Test helper: wait until a job queue has nothing left to do.

use std::time::{Duration, Instant};

use super::JobQueue;

/// How long a test waits for its jobs before it gives up and says which
/// were still going. Long enough that a passing test never reaches it,
/// however busy the machine; a hung one ends here with a message instead
/// of at the CI job's limit.
const PATIENCE: Duration = Duration::from_secs(300);

/// Waits until no job is queued or running, and the queue has recorded
/// nothing new since it was last seen that way.
///
/// A chained job queues what follows it before it ends, so an empty list
/// means the chain is over. Because the empty list is the only thing the
/// test sees, it takes two looks: the list is empty, and a moment later it
/// is still empty and the queue's `seq` (its count of recorded changes)
/// hasn't moved. A job queued in the gap between one job ending and the next
/// being asked for would move `seq`, so it can't be mistaken for the end.
pub(crate) fn wait_until_idle(queue: &JobQueue) {
    let start = Instant::now();
    let mut seen_empty_at = None;
    loop {
        let snapshot = queue.activity().unwrap();
        if snapshot.jobs.is_empty() {
            if seen_empty_at == Some(snapshot.seq) {
                return;
            }
            seen_empty_at = Some(snapshot.seq);
        } else {
            seen_empty_at = None;
        }
        assert!(
            start.elapsed() < PATIENCE,
            "the jobs never finished within {PATIENCE:?}; still going: {:?}",
            snapshot
                .jobs
                .iter()
                .map(|j| (j.id, j.kind, j.status))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
