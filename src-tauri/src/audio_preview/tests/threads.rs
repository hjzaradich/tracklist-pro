//! Where the work runs.

use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::audio_preview::serve::{contained, guarded, refused};
use crate::audio_preview::workers::{Workers, THREADS};
use crate::audio_preview::PreviewRefusal;

#[test]
fn a_request_is_answered_on_a_worker_thread_never_the_one_that_handed_it_over() {
    let workers = Workers::new();
    let (send, receive) = channel();
    workers.run(move || {
        send.send(thread::current().name().map(str::to_owned))
            .unwrap();
    });
    let name = receive.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_ne!(thread::current().name(), name.as_deref());
    assert!(name.unwrap().starts_with("audio-preview-"));
}

#[test]
fn at_most_a_few_requests_are_worked_on_at_once() {
    let workers = Workers::new();
    let (send, receive) = channel();
    let (release, wait) = channel::<()>();
    let wait = Arc::new(Mutex::new(wait));
    for _ in 0..THREADS + 3 {
        let (send, wait) = (send.clone(), wait.clone());
        workers.run(move || {
            send.send(()).unwrap();
            let _ = wait.lock().unwrap().recv();
        });
    }
    // THREADS of them start; the rest wait in line.
    for _ in 0..THREADS {
        receive.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    assert!(receive.recv_timeout(Duration::from_millis(300)).is_err());
    drop(release);
    for _ in 0..3 {
        receive.recv_timeout(Duration::from_secs(10)).unwrap();
    }
}

#[test]
fn a_decoder_that_panics_is_a_file_that_cannot_be_decoded_and_costs_no_worker() {
    let panicked: Result<(), PreviewRefusal> = contained(|| panic!("a decoder gave up"));
    assert_eq!(panicked, Err(PreviewRefusal::NotDecodable));
    assert_eq!(contained(|| Ok(5)), Ok(5));
    assert_eq!(
        contained::<()>(|| Err(PreviewRefusal::Missing)),
        Err(PreviewRefusal::Missing)
    );

    // A job that panics doesn't end the thread that ran it.
    let workers = Workers::new();
    for _ in 0..THREADS * 2 {
        workers.run(|| panic!("a request went wrong"));
    }
    let (send, receive) = channel();
    workers.run(move || send.send(()).unwrap());
    receive.recv_timeout(Duration::from_secs(10)).unwrap();
}

#[test]
fn a_check_that_panics_answers_unreadable_and_a_request_that_panics_still_gets_a_reply() {
    // The command's guard: a panic in the probe is a refusal the player can
    // show, not a reply that never comes.
    assert_eq!(
        super::super::check(|| panic!("a probe gave up")),
        Some(PreviewRefusal::Unreadable)
    );
    assert_eq!(super::super::check(|| Ok(())), None);
    assert_eq!(
        super::super::check(|| Err(PreviewRefusal::Missing)),
        Some(PreviewRefusal::Missing)
    );
    // The handler's: anything that builds the reply can panic, and the
    // request is answered with a 500.
    let reply = guarded(|| panic!("the volumes could not be read"));
    assert_eq!(reply.status().as_u16(), 500);
    assert_eq!(
        guarded(|| refused(PreviewRefusal::Missing))
            .status()
            .as_u16(),
        404
    );
}
