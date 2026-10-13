//! A few worker threads for preview requests, so the webview thread only
//! hands them over. Bounded, so a page asking for many ranges at once
//! (scrubbing) can't start many readers: the rest wait in line.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;

/// How many requests are answered at once.
pub(super) const THREADS: usize = 4;

type Work = Box<dyn FnOnce() + Send>;

pub(super) struct Workers {
    /// Started with the first request, so an app that never previews has no
    /// extra threads.
    queue: OnceLock<Sender<Work>>,
}

impl Workers {
    pub fn new() -> Workers {
        Workers {
            queue: OnceLock::new(),
        }
    }

    /// Runs `work` on a worker thread, after the ones queued before it.
    pub fn run(&self, work: impl FnOnce() + Send + 'static) {
        let queue = self.queue.get_or_init(|| {
            let (send, receive) = channel::<Work>();
            let receive: Arc<Mutex<Receiver<Work>>> = Arc::new(Mutex::new(receive));
            for n in 0..THREADS {
                let receive = receive.clone();
                let started = thread::Builder::new()
                    .name(format!("audio-preview-{n}"))
                    .spawn(move || loop {
                        // Take one job, then let go of the line.
                        let next = receive
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .recv();
                        match next {
                            Ok(work) => {
                                // One request failing must not cost a thread.
                                let _ = catch_unwind(AssertUnwindSafe(work));
                            }
                            // The sender is gone: the app is.
                            Err(_) => return,
                        }
                    });
                if let Err(e) = started {
                    eprintln!("audio preview: couldn't start a worker thread: {e}");
                }
            }
            send
        });
        // A send fails only with every worker gone; the request then goes
        // unanswered, which the page sees as a failed load.
        let _ = queue.send(Box::new(work));
    }
}
