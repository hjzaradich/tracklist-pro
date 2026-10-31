//! A send's review holds the writer as little as it can (1cD-S1), and is
//! still of one moment of the database: the moment the matching (or a merge
//! that wrote) ended. Changes landing during a review don't reach it, and
//! refuse the go exactly as a change made after the prepare does. Every name
//! is invented.

use std::sync::mpsc::{self, Sender as Channel};
use std::sync::{Arc, Mutex};

use super::*;
use crate::conflicts::Merged;
use crate::jobs::JobId;
use crate::library::LibraryTrackId;
use crate::send::job::Sender;
use crate::send::preflight::{merge, merge_read_only, HasWrites};

/// An open conflict on a field whose sides don't disagree: the merge
/// withdraws it, so it has something to write.
fn a_conflict_to_withdraw(w: &World, track: LibraryTrackId) {
    w.insert(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value)
         VALUES (?1, 'Artist', 'Mine', 'Theirs')",
        (track.0,),
    );
}

fn open_conflicts(w: &World) -> i64 {
    w.writer
        .call(|c| c.query_row("SELECT count(*) FROM conflict", [], |r| r.get(0)))
        .unwrap()
}

/// A track added from a hook, while a review runs.
fn add_a_track(writer: &Writer) {
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO recording (title) VALUES ('Late');
                 INSERT INTO library_track (recording_id, kind, source_status)
                 VALUES (last_insert_rowid(), 'linked', 'missing');",
            )
        })
        .unwrap();
}

/// Holds the writer from the moment a merge starts until it ends, so a merge
/// that needed the writer would wait the whole time. Says, for each merge,
/// whether it ended while the writer was held.
#[derive(Clone)]
struct WriterBusy {
    writer: Writer,
    release: Arc<Mutex<Option<Channel<()>>>>,
    finished_meanwhile: Arc<Mutex<Vec<bool>>>,
}

impl WriterBusy {
    fn new(writer: &Writer) -> WriterBusy {
        WriterBusy {
            writer: writer.clone(),
            release: Arc::default(),
            finished_meanwhile: Arc::default(),
        }
    }

    fn start(&self) {
        let (release, released) = mpsc::channel::<()>();
        *self.release.lock().unwrap() = Some(release);
        let (held, is_held) = mpsc::channel::<()>();
        let (writer, seen) = (self.writer.clone(), self.finished_meanwhile.clone());
        std::thread::spawn(move || {
            writer
                .call(move |_| {
                    held.send(()).unwrap();
                    let ended = released.recv_timeout(Duration::from_secs(15));
                    seen.lock().unwrap().push(ended.is_ok());
                    Ok(())
                })
                .unwrap();
        });
        is_held.recv().unwrap();
    }

    fn stop(&self) {
        if let Some(release) = self.release.lock().unwrap().take() {
            release.send(()).unwrap();
        }
    }

    fn on(&self, sender: Sender<Plugged>) -> Sender<Plugged> {
        let (start, stop) = (self.clone(), self.clone());
        sender
            .before_merge(move || start.start())
            .after_merge(move || stop.stop())
    }
}

#[test]
fn a_merge_with_nothing_to_write_goes_through_while_the_writer_does_something_else() {
    let (w, ..) = world_with_a_send_waiting();
    let busy = WriterBusy::new(&w.writer);

    let sender = busy.on(w.sender());
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);
    assert_eq!(*busy.finished_meanwhile.lock().unwrap(), [true]);

    // The go's merge as well.
    let token = w.flow.preflight().unwrap().token;
    let sender = busy.on(w.sender());
    assert_eq!(w.go_with(sender, &token, false), None);
    assert_eq!(*busy.finished_meanwhile.lock().unwrap(), [true, true]);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn a_merge_read_only_is_the_merge_as_it_stands_and_writes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    let path = w.writer.guarded_path();

    let moment = crate::db::Moment::open(path).unwrap();
    moment.hold().unwrap();
    let read_only = merge_read_only(moment.conn(), &Plugged).unwrap();
    drop(moment);
    let on_the_writer = w.writer.call(|c| merge(c, &Plugged)).unwrap();
    assert_eq!(read_only, Ok(on_the_writer));
}

#[test]
fn a_merge_read_only_says_so_when_it_has_something_to_write_and_changes_nothing() {
    let (w, known, _) = world_with_a_send_waiting();
    w.prepare().unwrap();
    a_conflict_to_withdraw(&w, known);

    let moment = crate::db::Moment::open(w.writer.guarded_path()).unwrap();
    moment.hold().unwrap();
    let read_only = merge_read_only(moment.conn(), &Plugged).unwrap();
    assert_eq!(read_only, Err(HasWrites));
    assert_eq!(open_conflicts(&w), 1);
}

#[test]
fn a_merge_with_something_to_write_writes_it_and_the_preflight_is_of_what_it_wrote() {
    let (w, known, _) = world_with_a_send_waiting();
    w.insert(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value)
         VALUES (?1, 'Artist', 'Mine', 'Theirs')",
        (known.0,),
    );
    let preflight = w.prepare().unwrap();
    assert_eq!(open_conflicts(&w), 0);
    assert_eq!(preflight.conflicts_waiting, 0);
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_eq!(w.go(false), None);
}

#[test]
fn a_go_whose_merge_has_something_to_write_writes_it_and_still_sends_the_reviewed_file() {
    let (w, known, _) = world_with_a_send_waiting();
    w.prepare().unwrap();
    a_conflict_to_withdraw(&w, known);

    assert_eq!(w.go(false), None);
    assert_eq!(open_conflicts(&w), 0);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn a_change_landing_before_the_merge_is_not_in_the_preflight_and_refuses_the_go() {
    let (w, ..) = world_with_a_send_waiting();
    let writer = w.writer.clone();
    let sender = w.sender().before_merge(move || add_a_track(&writer));
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    // The preflight is of the moment the matching ended.
    let preflight = w.flow.preflight().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_eq!(preflight.left_out, []);
    // A send of what the library holds now would differ from it.
    assert_eq!(w.go(true), Some(SendFailure::LibraryChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
    assert_eq!(w.flow.preflight(), None);
}

#[test]
fn a_change_landing_after_the_merge_is_not_in_the_preflight_and_refuses_the_go() {
    let (w, ..) = world_with_a_send_waiting();
    let writer = w.writer.clone();
    let sender = w.sender().after_merge(move || add_a_track(&writer));
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    let preflight = w.flow.preflight().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_eq!(preflight.left_out, []);
    assert_eq!(w.go(true), Some(SendFailure::LibraryChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
}

#[test]
fn a_title_changed_during_the_review_never_shows_in_the_preflight_and_refuses_the_go() {
    // A file's name in a list comes from a read after the merge: it reads
    // the same moment as the rest, and the token is of that moment.
    let w = World::new();
    let (gone, _) = w.library_track("gone.mp3", "Before", false);
    w.crate_of("Warm up", &[gone]);
    w.save_export(&[]);
    let writer = w.writer.clone();
    let sender = w.sender().after_merge(move || {
        writer
            .call(|c| c.execute("UPDATE recording SET title = 'After'", []))
            .unwrap();
    });
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    let preflight = w.flow.preflight().unwrap();
    assert_eq!(preflight.left_out[0].track.title.as_deref(), Some("Before"));
    assert_eq!(w.go(true), Some(SendFailure::LibraryChanged));
}

#[test]
fn a_read_landing_right_after_the_matching_leaves_the_preflight_of_the_read_before_it() {
    let (w, ..) = world_with_a_send_waiting();
    let (writer, text) = (w.writer.clone(), fs::read_to_string(&w.export).unwrap());
    let sender = w.sender().before_merge(move || {
        another_read(&writer, text.clone(), None, "2031-01-01T00:00:00.000Z");
    });
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    // Its rows are unmatched, and the preflight never saw them: the track
    // rekordbox has doesn't count as new.
    let preflight = w.flow.preflight().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_ne!(preflight.export.read_at, "2031-01-01T00:00:00.000Z");
    // The go is of another read now, and says so.
    assert_eq!(w.go(false), Some(SendFailure::ExportChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
}

#[test]
fn a_read_landing_after_the_matching_of_a_merge_that_writes_is_matched_before_the_merge() {
    // The merge has to run again, on the writer, on rows the read left
    // unmatched: it matches them first, so no track rekordbox has goes out
    // as new.
    let (w, known, _) = world_with_a_send_waiting();
    a_conflict_to_withdraw(&w, known);
    let (writer, text) = (w.writer.clone(), fs::read_to_string(&w.export).unwrap());
    let sender = w.sender().before_merge(move || {
        another_read(&writer, text.clone(), None, "2031-01-01T00:00:00.000Z");
    });
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    let preflight = w.flow.preflight().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_eq!(preflight.export.read_at, "2031-01-01T00:00:00.000Z");
    assert_eq!(open_conflicts(&w), 0);
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn a_go_with_no_read_left_says_rekordbox_was_read_again() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    w.writer
        .call(|c| {
            c.execute(
                "DELETE FROM setting WHERE key = 'rekordbox_xml_last_read'",
                [],
            )
        })
        .unwrap();
    assert_eq!(w.go(false), Some(SendFailure::ExportChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
}

/// The send job's progress now, as stored.
fn progress_of_the_last_job(writer: &Writer) -> Option<f64> {
    writer
        .call(|c| {
            c.query_row(
                "SELECT progress FROM job WHERE kind = 'export' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
}

#[test]
fn a_send_names_each_part_of_its_two_steps_and_its_progress_only_goes_forward() {
    let (w, ..) = world_with_a_send_waiting();
    let seen: Arc<Mutex<Vec<(&'static str, Option<SendWork>, Option<f64>)>>> = Arc::default();
    let note = |at: &'static str| {
        let (flow, writer, seen) = (w.flow.clone(), w.writer.clone(), seen.clone());
        move || {
            let work = flow.state(None).working_on;
            let progress = progress_of_the_last_job(&writer);
            seen.lock().unwrap().push((at, work, progress));
        }
    };
    let (flow, matched) = (w.flow.clone(), seen.clone());
    let sender = w
        .sender()
        .after_relink(move || {
            // On the writer's own thread: only what the flow says.
            let work = flow.state(None).working_on;
            matched.lock().unwrap().push(("matched", work, None));
        })
        .before_merge(note("merge starts"))
        .after_merge(note("merge ends"));
    assert_eq!(
        w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy())),
        JobStatus::Done
    );
    assert_eq!(w.flow.state(None).working_on, None);
    let token = w.flow.preflight().unwrap().token;

    let (flow, recording) = (w.flow.clone(), seen.clone());
    let sender = w
        .sender()
        .before_merge(note("go's merge starts"))
        .before_write(note("go's file"))
        .recording_with(move |c, sent, paths| {
            let work = flow.state(None).working_on;
            recording.lock().unwrap().push(("recording", work, None));
            crate::rekordbox_write::record_send(c, sent, paths)
        });
    assert_eq!(w.go_with(sender, &token, false), None);
    assert_eq!(w.flow.state(None).working_on, None);

    let seen = seen.lock().unwrap();
    let parts: Vec<(&str, Option<SendWork>)> =
        seen.iter().map(|&(at, work, _)| (at, work)).collect();
    assert_eq!(
        parts,
        [
            ("matched", Some(SendWork::Matching)),
            ("merge starts", Some(SendWork::Merging)),
            ("merge ends", Some(SendWork::Merging)),
            ("go's merge starts", Some(SendWork::Merging)),
            ("go's file", Some(SendWork::Building)),
            ("recording", Some(SendWork::Recording)),
        ]
    );
    // Each step's own job reports as it goes, and only forward.
    let prepare: Vec<f64> = seen[1..3].iter().filter_map(|s| s.2).collect();
    assert_eq!(prepare.len(), 2, "{seen:?}");
    assert!(prepare[0] > 0.0 && prepare[0] <= prepare[1] && prepare[1] < 1.0);
    let go: Vec<f64> = seen[3..5].iter().filter_map(|s| s.2).collect();
    assert_eq!(go.len(), 2, "{seen:?}");
    assert!(go[0] > 0.0 && go[0] < go[1] && go[1] < 1.0);
}

#[test]
fn a_prepare_cancelled_between_its_parts_stops_there_and_leaves_no_preflight() {
    let (w, ..) = world_with_a_send_waiting();
    let (reached, at) = mpsc::channel::<()>();
    let (go_on, resume) = mpsc::channel::<()>();
    let (reached, resume) = (Mutex::new(reached), Mutex::new(resume));
    let sender = w.sender().after_merge(move || {
        reached.lock().unwrap().send(()).unwrap();
        let _ = resume.lock().unwrap().recv_timeout(Duration::from_secs(30));
    });
    let queue = JobQueue::builder(w.writer.clone())
        .workers(1)
        .handler(JobKind::Export, sender)
        .start()
        .unwrap();
    let id: JobId = queue
        .enqueue(w.flow.prepare_job(&w.export.to_string_lossy()))
        .unwrap();
    at.recv_timeout(Duration::from_secs(60)).unwrap();
    queue.cancel(id).unwrap();
    go_on.send(()).unwrap();
    let start = Instant::now();
    let status = loop {
        let job = w
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            break job.status;
        }
        assert!(start.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
    };
    queue.shutdown();
    assert_eq!(status, JobStatus::Cancelled);
    assert_eq!(w.flow.failure(), Some(SendFailure::Cancelled));
    assert_eq!(w.flow.preflight(), None);
    assert_eq!(w.flow.state(None).working_on, None);
}

#[test]
fn merging_on_a_moment_gives_what_the_writer_gives_for_a_send_with_conflicts_open() {
    // A field changed on both sides stays open: the merge has nothing more
    // to write, and says the same on a read-only connection.
    let (w, known, _) = world_with_a_send_waiting();
    w.writer
        .call(move |c| {
            Ok(crate::conflicts::app_values::set(
                c, &Plugged, known, "Artist", "Mine",
            ))
        })
        .unwrap()
        .unwrap();
    w.prepare().unwrap();
    let merged_now = |w: &World| -> Merged {
        let moment = crate::db::Moment::open(w.writer.guarded_path()).unwrap();
        moment.hold().unwrap();
        let ids = crate::library::stored_ids(moment.conn()).unwrap();
        crate::conflicts::merge_without_writing(moment.conn(), &Plugged, &ids)
            .unwrap()
            .expect("nothing to write")
    };
    let ids = w.writer.call(|c| crate::library::stored_ids(c)).unwrap();
    let on_the_writer = w
        .writer
        .call(move |c| crate::conflicts::merge_before_send(c, &Plugged, &ids))
        .unwrap();
    assert_eq!(merged_now(&w), on_the_writer);
}
