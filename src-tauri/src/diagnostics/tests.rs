//! Behavior tests for the event log and the report: the ring's bounds, what
//! never gets in, and where the files go.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use super::commands::{preview, save, view};
use super::report::{self, ReportDraft};
use super::*;
use crate::db::{ReadPool, Writer, DB_FILE_NAME};
use crate::jobs::{JobError, JobQueue, NewJob};

/// A message that holds what must never reach the log or the report: a
/// path with the user's account name, and a track name.
const LEAKY: &str = r"C:\Users\dj_alex\Music\Daft Punk - One More Time (Secret Edit).mp3: no good";

fn guard(dir: &Path) -> WriteGuard {
    WriteGuard::app_data(dir).unwrap()
}

fn events_file(dir: &Path) -> String {
    fs::read_to_string(dir.join(EVENTS_FILE)).unwrap_or_default()
}

fn small(events: usize, bytes: usize) -> Limits {
    Limits { events, bytes }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting until {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A database in `dir` with a writer and a read pool.
fn database(dir: &Path) -> (Writer, ReadPool) {
    let writer = Writer::open(&crate::write_guard::test_path(dir, DB_FILE_NAME)).unwrap();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    (writer, reads)
}

fn set(writer: &Writer, key: &'static str, value: serde_json::Value) {
    writer
        .call(move |c| {
            c.execute(
                "INSERT INTO setting (key, value) VALUES (?1, json(?2))",
                (key, value.to_string()),
            )
        })
        .unwrap();
}

/// A queue with one worker whose `kind` jobs run `handler`.
fn queue(
    writer: &Writer,
    log: &EventLog,
    handler: impl Fn(&crate::jobs::JobContext) -> Result<(), JobError> + Send + Sync + 'static,
) -> JobQueue {
    JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, handler)
        .events(log.clone())
        .start()
        .unwrap()
}

fn kinds(log: &EventLog) -> Vec<EventKind> {
    log.events().iter().map(|e| e.kind).collect()
}

#[test]
fn the_ring_keeps_the_newest_events_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let limits = small(5, MAX_BYTES);
    let log = EventLog::open_with(&guard(dir.path()), limits);
    for n in 0..12 {
        log.record(LogEvent::new(EventKind::JobStarted).at(n));
    }
    let at: Vec<i64> = log.events().iter().map(|e| e.at_ms).collect();
    assert_eq!(at, [7, 8, 9, 10, 11]);
    assert_eq!(view(&log).events[0].at_ms, 11, "the view is newest first");
    // The file may be a tenth past the limit between rewrites, and no more...
    assert!(events_file(dir.path()).lines().count() <= 6);
    // ...and read back it is the same ring.
    let again = EventLog::open_with(&guard(dir.path()), limits);
    assert_eq!(again.events(), log.events());
}

#[test]
fn the_ring_stays_under_its_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let limits = small(MAX_EVENTS, 600);
    let log = EventLog::open_with(&guard(dir.path()), limits);
    for n in 0..100 {
        log.record(
            LogEvent::new(EventKind::JobFailed)
                .job(JobKind::Scan)
                .reason(EventReason::JobFailed)
                .at(n),
        );
        let ring: usize = log
            .events()
            .iter()
            .map(|e| serde_json::to_string(e).unwrap().len() + 1)
            .sum();
        assert!(ring <= 600, "{ring} bytes in the ring");
        // The file: the limit and the tenth it may run over by.
        assert!(events_file(dir.path()).len() <= 660);
    }
    let kept = log.events();
    assert!(kept.len() > 1);
    assert_eq!(kept.last().unwrap().at_ms, 99, "the newest is kept");
    let again = EventLog::open_with(&guard(dir.path()), limits);
    assert_eq!(again.events(), kept, "read back within the limit");
}

#[test]
fn an_event_is_added_to_the_file_not_the_whole_file_written_again() {
    let dir = tempfile::tempdir().unwrap();
    let log = EventLog::open(&guard(dir.path()));
    let mut before = String::new();
    for n in 0..20 {
        log.record(LogEvent::new(EventKind::JobStarted).at(n));
        let now = events_file(dir.path());
        assert!(now.starts_with(&before), "event {n} changed what was there");
        assert_eq!(now.lines().count(), n as usize + 1);
        before = now;
    }
}

#[test]
fn a_rewrite_replaces_the_file_in_one_step_and_the_log_is_never_empty() {
    let dir = tempfile::tempdir().unwrap();
    let log = EventLog::open_with(&guard(dir.path()), small(10, MAX_BYTES));
    for n in 0..100 {
        log.record(LogEvent::new(EventKind::JobStarted).at(n));
        let file = events_file(dir.path());
        assert!(!file.is_empty(), "the file was empty after event {n}");
        for line in file.lines() {
            serde_json::from_str::<LogEvent>(line).unwrap();
        }
        assert!(file.lines().count() <= 11, "event {n}");
    }
    // No half-made file left beside it.
    let beside: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".part"))
        .collect();
    assert!(beside.is_empty(), "{beside:?}");
}

#[test]
fn a_rewrite_that_a_crash_cut_short_leaves_the_old_log_and_its_leftover_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let first = EventLog::open(&guard(dir.path()));
    first.record(LogEvent::new(EventKind::Startup).at(1));
    first.record(LogEvent::new(EventKind::JobStarted).at(2));
    // What a crash between making the new file and renaming it leaves.
    let part = dir.path().join(format!("{EVENTS_FILE}.part"));
    fs::write(&part, "half").unwrap();
    let again = EventLog::open(&guard(dir.path()));
    assert_eq!(again.events().len(), 2);
    assert!(!part.exists());
}

#[test]
fn a_cut_off_last_line_is_dropped_and_the_next_event_starts_a_clean_line() {
    let dir = tempfile::tempdir().unwrap();
    let first = EventLog::open(&guard(dir.path()));
    first.record(LogEvent::new(EventKind::Startup).at(1));
    let mut file = events_file(dir.path());
    file.push_str("{\"atMs\":2,\"kind\":\"job");
    fs::write(dir.path().join(EVENTS_FILE), file).unwrap();
    let second = EventLog::open(&guard(dir.path()));
    assert_eq!(second.events().len(), 1);
    second.record(LogEvent::new(EventKind::JobStarted).at(3));
    let third = EventLog::open(&guard(dir.path()));
    let at: Vec<i64> = third.events().iter().map(|e| e.at_ms).collect();
    assert_eq!(at, [1, 3]);
    assert!(events_file(dir.path())
        .lines()
        .all(|l| l.starts_with('{') && l.ends_with('}')));
}

#[test]
fn a_file_far_past_the_limit_is_not_read_and_is_replaced_by_the_next_event() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(EVENTS_FILE), "x".repeat(5 * MAX_BYTES)).unwrap();
    let log = EventLog::open(&guard(dir.path()));
    assert!(log.events().is_empty());
    log.record(LogEvent::new(EventKind::Startup).at(1));
    assert_eq!(events_file(dir.path()).lines().count(), 1);
}

#[test]
fn a_failed_write_is_made_good_by_a_rewrite_at_the_next_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(EVENTS_FILE);
    fs::create_dir(&path).unwrap();
    let log = EventLog::open(&guard(dir.path()));
    log.record(LogEvent::new(EventKind::Startup).at(1));
    assert_eq!(log.lost_writes(), 1);
    fs::remove_dir(&path).unwrap();
    log.record(LogEvent::new(EventKind::JobStarted).at(2));
    assert_eq!(log.lost_writes(), 1);
    let again = EventLog::open(&guard(dir.path()));
    let at: Vec<i64> = again.events().iter().map(|e| e.at_ms).collect();
    assert_eq!(
        at,
        [1, 2],
        "the event that failed to write is in the file too"
    );
}

#[test]
fn the_real_limits_are_two_thousand_events_and_about_a_megabyte() {
    assert_eq!(Limits::default().events, 2_000);
    assert_eq!(Limits::default().bytes, 1_000_000);
}

#[test]
fn the_ring_survives_a_restart_and_drops_lines_that_are_not_events() {
    let dir = tempfile::tempdir().unwrap();
    let first = EventLog::open(&guard(dir.path()));
    first.record(LogEvent::new(EventKind::Startup).at(1).count(2));
    // A line with a field the allow-list doesn't have, and a line of junk.
    let mut file = events_file(dir.path());
    file.push_str(
        "{\"atMs\":2,\"kind\":\"jobFailed\",\"job\":null,\"reason\":null,\"count\":null,\"ms\":null,\"path\":\"C:\\\\x\"}\n",
    );
    file.push_str("not an event\n");
    fs::write(dir.path().join(EVENTS_FILE), file).unwrap();
    let second = EventLog::open(&guard(dir.path()));
    let events = second.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].at_ms, 1);
    assert_eq!(events[0].count, Some(2));
}

#[test]
fn a_failed_job_is_logged_by_kind_and_reason_never_by_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let log = EventLog::open(&guard(dir.path()));
    let (writer, _reads) = database(dir.path());
    let queue = queue(&writer, &log, |_| Err(JobError::failed(LEAKY)));
    queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    wait_until("the failure is logged", || {
        kinds(&log).contains(&EventKind::JobFailed)
    });
    let failed = log
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::JobFailed)
        .unwrap();
    assert_eq!(failed.job, Some(JobKind::Scan));
    assert_eq!(failed.reason, Some(EventReason::JobFailed));
    assert!(failed.ms.is_some());
    assert_eq!(kinds(&log)[0], EventKind::JobStarted);
    let file = events_file(dir.path());
    for secret in [
        "Daft Punk",
        "dj_alex",
        "Secret Edit",
        "C:",
        ".mp3",
        "no good",
    ] {
        assert!(!file.contains(secret), "{secret} reached the log: {file}");
    }
}

#[test]
fn a_panicking_job_is_logged_as_panicked_without_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let log = EventLog::open(&guard(dir.path()));
    let (writer, _reads) = database(dir.path());
    let queue = queue(&writer, &log, |_| panic!("{LEAKY}"));
    queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    wait_until("the panic is logged", || {
        kinds(&log).contains(&EventKind::JobFailed)
    });
    let failed = log
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::JobFailed)
        .unwrap();
    assert_eq!(failed.reason, Some(EventReason::Panicked));
    assert!(!events_file(dir.path()).contains("Daft Punk"));
}

#[test]
fn a_finished_job_is_logged_with_how_long_it_took() {
    let dir = tempfile::tempdir().unwrap();
    let log = EventLog::open(&guard(dir.path()));
    let (writer, _reads) = database(dir.path());
    let queue = queue(&writer, &log, |job| job.check_cancelled());
    queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    wait_until("the job is logged finished", || {
        kinds(&log).contains(&EventKind::JobFinished)
    });
    let done = log
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::JobFinished)
        .unwrap();
    assert_eq!(done.job, Some(JobKind::Scan));
    assert!(done.ms.is_some() && done.reason.is_none());
}

#[test]
fn a_log_that_cannot_be_written_does_not_fail_the_job() {
    let dir = tempfile::tempdir().unwrap();
    // A folder where the log's file should be: every write to it fails.
    fs::create_dir(dir.path().join(EVENTS_FILE)).unwrap();
    let log = EventLog::open(&guard(dir.path()));
    let (writer, _reads) = database(dir.path());
    let queue = queue(&writer, &log, |_| Ok(()));
    let id = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    wait_until("the job is done", || {
        let job = writer
            .call(move |c| crate::jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        job.status == crate::jobs::JobStatus::Done
    });
    assert!(log.lost_writes() > 0, "the failed writes are counted");
    // The ring still has the events, for this run.
    assert!(kinds(&log).contains(&EventKind::JobFinished));
}

#[test]
fn the_report_counts_what_the_database_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, reads) = database(dir.path());
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO crate (kind, name) VALUES ('static', 'A'), ('static', 'B'), ('folder', 'C');
                 INSERT INTO playlist (name, name_key) VALUES ('P1', 'p1'), ('P2', 'p2');
                 INSERT INTO session (source, name, played_on)
                     VALUES ('m3u8', 'S1', '2026-01-01'), ('m3u8', 'S2', '2026-01-02'),
                            ('stick', 'S3', '2026-01-03'), ('stick', 'S4', '2026-01-04');",
            )
        })
        .unwrap();
    let log = EventLog::off();
    let text = report::build(&reads, &log, 0).unwrap();
    for line in [
        "crates: 3",
        "playlists: 2",
        "sessions: 4",
        "library_tracks: 0",
        "files: 0",
        "version: ",
        "schema: version ",
        "wishlist: off",
        "rekordbox_database: off",
        "musicbrainz: off",
        "discogs: off",
        "created: 1970-01-01T00:00:00.000Z",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    let applied = crate::db::migrations::MIGRATIONS.len();
    assert!(text.contains(&format!("{applied} migrations applied")));
}

#[test]
fn the_report_lists_features_failures_and_events_by_code() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, reads) = database(dir.path());
    writer
        .call(|c| {
            crate::settings::write_feature_enabled(
                c,
                crate::settings::Feature::RekordboxDatabase,
                true,
            )
        })
        .unwrap();
    let log = EventLog::open(&guard(dir.path()));
    log.record(
        LogEvent::new(EventKind::RekordboxReadFailed)
            .job(JobKind::ReadRekordbox)
            .reason(EventReason::Damaged)
            .at(1_700_000_000_000),
    );
    log.record(LogEvent::new(EventKind::DatabaseReadFailed).reason(DbFailure::WrongKey));
    log.record(
        LogEvent::new(EventKind::JobFailed)
            .job(JobKind::Hash)
            .reason(EventReason::Panicked),
    );
    let text = report::build(&reads, &log, 0).unwrap();
    assert!(text.contains("rekordbox_database: on"), "{text}");
    assert!(text.contains(
        "2023-11-14T22:13:20.000Z rekordboxReadFailed job=read_rekordbox reason=damaged"
    ));
    assert!(text.contains("database last read: failed, wrongKey"));
    assert!(text.contains("job=hash panicked: 1"), "{text}");
}

#[test]
fn the_free_space_line_is_a_whole_number_of_gb_and_nothing_else() {
    const GB: u64 = 1 << 30;
    assert_eq!(report::free_space_line(Some(0)), "disk free: 0 GB");
    assert_eq!(report::free_space_line(Some(GB / 2 - 1)), "disk free: 0 GB");
    assert_eq!(report::free_space_line(Some(GB / 2)), "disk free: 1 GB");
    assert_eq!(
        report::free_space_line(Some(250 * GB + GB / 4)),
        "disk free: 250 GB"
    );
    assert_eq!(report::free_space_line(None), "disk free: unknown");
}

#[test]
fn the_report_holds_no_setting_value_path_name_or_key() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, reads) = database(dir.path());
    let xml = r"C:\Users\dj_alex\Documents\Secret Export Path.xml";
    // The rows the XML source keeps: its path, its last read and its last
    // failure, each with the path in it.
    set(&writer, "rekordbox_xml_path", serde_json::json!(xml));
    set(
        &writer,
        "rekordbox_xml_last_failure",
        serde_json::json!({ "path": xml, "reason": "damaged", "at": "2026-01-01T00:00:00Z" }),
    );
    set(
        &writer,
        "rekordbox_xml_last_read",
        serde_json::json!({
            "path": xml, "modifiedMs": 1, "readAt": "2026-01-01T00:00:00Z",
            "summary": { "tracks": 7, "streaming": 1, "kept": 0, "notStored": 0, "complete": true },
            "source": "xml"
        }),
    );
    // Rows the report doesn't know: any name, any value.
    set(
        &writer,
        "rekordbox_database_path",
        serde_json::json!(r"D:\Private Folder\master.db"),
    );
    set(
        &writer,
        "discogs_token",
        serde_json::json!("tok-9f8e7d6c5b4a"),
    );
    set(
        &writer,
        "unlisted_secret_setting",
        serde_json::json!("Hunter2 Key Value"),
    );
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO crate (kind, name) VALUES ('static', 'Peak Time Banger Crate');
                 INSERT INTO playlist (name, name_key) VALUES ('Warmup Secret Playlist', 'w');
                 INSERT INTO session (source, name, played_on)
                     VALUES ('m3u8', 'Saturday At Club Nowhere', '2026-01-01');",
            )
        })
        .unwrap();
    // A failed job whose stored error holds a path and a track name.
    let log = EventLog::open(&guard(dir.path()));
    let queue = queue(&writer, &log, |_| Err(JobError::failed(LEAKY)));
    queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    wait_until("the failure is logged", || {
        kinds(&log).contains(&EventKind::JobFailed)
    });
    wait_until("the failure is stored", || {
        writer
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM job WHERE status = 'failed'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
            })
            .unwrap()
            == 1
    });
    let text = report::build(&reads, &log, 0).unwrap();
    // What it does say about them: a reason code and counts.
    assert!(text.contains("xml last failure: damaged"), "{text}");
    assert!(text.contains("7 tracks"), "{text}");
    assert!(text.contains("failed: scan=1"), "{text}");
    // The free-space line is a number of GB or "unknown": no drive letter,
    // volume name or path can ride along.
    let line = text
        .lines()
        .find(|l| l.starts_with("disk free:"))
        .unwrap_or_else(|| {
            panic!(
                "no disk free line:
{text}"
            )
        });
    let shown = line.trim_start_matches("disk free: ");
    let number = shown.strip_suffix(" GB");
    assert!(
        shown == "unknown"
            || number.is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())),
        "{line}"
    );
    let data = dir.path().to_string_lossy().into_owned();
    for secret in [
        "Secret Export Path",
        "dj_alex",
        "Documents",
        "Private Folder",
        "master.db",
        "tok-9f8e7d6c5b4a",
        "Hunter2",
        "unlisted_secret_setting",
        "discogs_token",
        "rekordbox_xml_path",
        "Peak Time",
        "Warmup Secret",
        "Club Nowhere",
        "Daft Punk",
        "Secret Edit",
        ".mp3",
        ".xml",
        "C:\\",
        DB_FILE_NAME,
        &data,
    ] {
        assert!(
            !text.contains(secret),
            "{secret:?} reached the report:\n{text}"
        );
    }
}

#[test]
fn a_job_row_of_a_kind_this_build_does_not_know_is_reported_as_other() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, reads) = database(dir.path());
    writer
        .call(|c| {
            c.execute(
                "INSERT INTO job (kind, status, error, finished_at)
                 VALUES (?1, 'failed', 'x', '2026-01-01T00:00:00Z')",
                [r"C:\Users\dj_alex\Daft Punk - One More Time.mp3"],
            )
        })
        .unwrap();
    let text = report::build(&reads, &EventLog::off(), 0).unwrap();
    assert!(text.contains("failed: other=1"), "{text}");
    for secret in ["dj_alex", "Daft Punk", ".mp3", "C:\\"] {
        assert!(
            !text.contains(secret),
            "{secret:?} reached the report:\n{text}"
        );
    }
}

#[test]
fn a_report_is_saved_in_the_app_data_folder_and_old_ones_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let guard = guard(dir.path());
    let reports = dir.path().join(REPORT_DIR);
    // Something else in the folder is not ours to delete.
    fs::create_dir(&reports).unwrap();
    fs::write(reports.join("notes.txt"), "mine").unwrap();
    let mut paths = Vec::new();
    for n in 0..8 {
        paths.push(report::save(&guard, &format!("report {n}"), 1_700_000_000_000 + n).unwrap());
    }
    for path in &paths {
        // The guard hands out paths in the folder's real, resolved form.
        assert!(path.starts_with(fs::canonicalize(&reports).unwrap()));
        assert_eq!(path.extension().unwrap(), "txt");
    }
    let mut left: Vec<String> = fs::read_dir(&reports)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left.len(), KEEP_REPORTS + 1, "{left:?}");
    assert!(left.contains(&"notes.txt".to_owned()));
    // The newest are kept.
    for path in &paths[8 - KEEP_REPORTS..] {
        assert!(path.exists(), "{path:?}");
    }
    assert_eq!(fs::read_to_string(&paths[7]).unwrap(), "report 7");
}

#[test]
fn what_the_preview_shows_is_what_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (_writer, reads) = database(dir.path());
    let guard = guard(dir.path());
    let log = EventLog::open(&guard);
    let draft = ReportDraft::default();
    // Nothing previewed, nothing to save.
    assert!(save(&guard, &draft).is_err());
    let shown = preview(&reads, &log, &draft).unwrap();
    let saved = save(&guard, &draft).unwrap();
    assert_eq!(fs::read_to_string(&saved.path).unwrap(), shown);
    let reports = fs::canonicalize(dir.path().join(REPORT_DIR)).unwrap();
    assert!(Path::new(&saved.path).starts_with(reports));
    // It is saved once: the draft is spent.
    assert!(save(&guard, &draft).is_err());
}

#[test]
fn the_app_logs_its_startup_and_serves_the_log_and_the_report_over_ipc() {
    use crate::ipc::testing::{app, invoke};
    use serde_json::json;
    use tauri::Manager;

    let (data, app) = app();
    let startup = app
        .state::<EventLog>()
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::Startup)
        .expect("startup is logged");
    assert!(startup.ms.is_some());
    let view = invoke(&app, "diagnostic_events", json!({})).unwrap();
    assert_eq!(view["events"][0]["kind"], "startup");
    // The log is a file in the app data folder.
    assert!(data.path().join("data").join(EVENTS_FILE).exists());

    let shown = invoke(&app, "preview_diagnostics_report", json!({})).unwrap();
    let saved = invoke(&app, "save_diagnostics_report", json!({})).unwrap();
    let path = std::path::PathBuf::from(saved["path"].as_str().unwrap());
    let reports = fs::canonicalize(data.path().join("data").join(REPORT_DIR)).unwrap();
    assert!(path.starts_with(reports));
    assert_eq!(fs::read_to_string(&path).unwrap(), shown.as_str().unwrap());
}

#[test]
fn times_are_written_in_utc() {
    assert_eq!(report::time(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(report::time(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    assert_eq!(report::time(951_782_400_000), "2000-02-29T00:00:00.000Z");
}
