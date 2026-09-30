//! 1aB-11: the XML source, the read job and the watch. Every fixture is
//! synthetic: made-up titles and paths only.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

use super::*;
use crate::db::Writer;
use crate::jobs::{self, JobStatus};

fn db() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    (dir, writer)
}

fn export(ids: &[u64]) -> String {
    let tracks: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"    <TRACK TrackID="{id}" Name="Synthetic {id}" Location="file://localhost/C:/Kit/{id}.mp3"/>"#
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="{}">
{}
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="0"/>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#,
        ids.len(),
        tracks.join("\n")
    )
}

/// Writes `text` to `path` and sets its modified time to `secs` after the
/// Unix epoch, so tests control which file is newer.
fn write_at(path: &Path, text: &str, secs: u64) {
    fs::write(path, text).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

/// A queue with one worker running the read job.
fn queue(writer: &Writer) -> JobQueue {
    JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::ReadRekordbox, XmlReader::default())
        .start()
        .unwrap()
}

/// Reads `path` as a job and waits for it to finish.
fn read_as_job(writer: &Writer, path: &Path) -> (JobStatus, Option<String>) {
    let queue = queue(writer);
    let id = queue.enqueue(read_job(path)).unwrap();
    let start = Instant::now();
    loop {
        let job = writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            queue.shutdown();
            return (job.status, job.error);
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the read never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn snapshot_ids(writer: &Writer) -> Vec<i64> {
    writer
        .call(|c| {
            c.prepare("SELECT track_id FROM rekordbox_track ORDER BY track_id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap()
}

/// Every stored row, whole, to prove a failed read changed nothing.
fn snapshot_rows(writer: &Writer) -> Vec<Vec<Option<String>>> {
    writer
        .call(|c| {
            c.prepare(
                "SELECT attributes, location_key, tempo, position_marks, playlists, read_at
                 FROM rekordbox_track ORDER BY id",
            )?
            .query_map([], |r| (0..6).map(|i| r.get(i)).collect())?
            .collect()
        })
        .unwrap()
}

/// The source as the screen gets it, with `folder` as Documents.
fn source(conn: &Connection, folder: Option<&Path>) -> rusqlite::Result<XmlSource> {
    Ok(offer_newer(
        stored_source(conn)?,
        folder,
        &NotExports::default(),
    ))
}

fn stored(writer: &Writer) -> XmlSource {
    writer.call(|c| stored_source(c)).unwrap()
}

// --- the read job ---

#[test]
fn reading_an_export_as_a_job_stores_the_snapshot_and_records_the_read() {
    let (dir, writer) = db();
    let xml = dir.path().join("rekordbox.xml");
    write_at(&xml, &export(&[1, 2, 3]), 1_000_000_000);
    let (status, _) = read_as_job(&writer, &xml);
    assert_eq!(status, JobStatus::Done);
    assert_eq!(snapshot_ids(&writer), [1, 2, 3]);
    let last = stored(&writer).last_read.unwrap();
    assert_eq!(last.path, xml.to_string_lossy());
    assert_eq!(last.modified_ms, 1_000_000_000_000);
    assert_eq!(last.summary.tracks, 3);
    assert!(last.summary.complete);
    // One read time for the whole read, and it's the one recorded.
    let read_at: Vec<String> = writer
        .call(|c| {
            c.prepare("SELECT DISTINCT read_at FROM rekordbox_track")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(read_at, [last.read_at]);
    // The file read is now the chosen export.
    assert_eq!(
        stored(&writer).path,
        Some(xml.to_string_lossy().into_owned())
    );
}

#[test]
fn a_malformed_or_cut_off_export_changes_nothing_and_says_it_is_damaged() {
    let (dir, writer) = db();
    let xml = dir.path().join("rekordbox.xml");
    write_at(&xml, &export(&[1, 2, 3]), 1_000_000_000);
    read_as_job(&writer, &xml);
    let rows = snapshot_rows(&writer);
    let last = stored(&writer).last_read;
    let chosen = stored(&writer).path;

    let full = export(&[1, 2]);
    let broken = [
        // Cut off mid-file, as when rekordbox is still writing it.
        full[..full.len() / 2].to_owned(),
        // A broken attribute.
        full.replace(r#"TrackID="2""#, "TrackID=2"),
    ];
    for text in broken {
        write_at(&xml, &text, 1_000_000_100);
        let (status, error) = read_as_job(&writer, &xml);
        assert_eq!(status, JobStatus::Failed);
        assert!(error.is_some());
        assert_eq!(snapshot_rows(&writer), rows);
        let source = stored(&writer);
        assert_eq!(source.last_read, last);
        assert_eq!(source.path, chosen);
        assert_eq!(source.last_failure.unwrap().reason, FailureReason::Damaged);
    }
}

#[test]
fn a_missing_file_or_one_that_is_not_an_export_fails_with_its_reason() {
    let (dir, writer) = db();
    let other = dir.path().join("other.xml");
    write_at(&other, r#"<?xml version="1.0"?><playlist/>"#, 1);
    for (path, reason) in [
        (dir.path().join("gone.xml"), FailureReason::NotFound),
        (other, FailureReason::NotAnExport),
    ] {
        let (status, _) = read_as_job(&writer, &path);
        assert_eq!(status, JobStatus::Failed);
        let failure = stored(&writer).last_failure.unwrap();
        assert_eq!(
            (failure.path, failure.reason),
            (path.to_string_lossy().into_owned(), reason)
        );
        assert!(snapshot_ids(&writer).is_empty());
    }
}

#[test]
fn a_successful_read_clears_the_last_failure() {
    let (dir, writer) = db();
    let xml = dir.path().join("rekordbox.xml");
    read_as_job(&writer, &xml);
    assert!(stored(&writer).last_failure.is_some());
    write_at(&xml, &export(&[7]), 5);
    assert_eq!(read_as_job(&writer, &xml).0, JobStatus::Done);
    assert_eq!(stored(&writer).last_failure, None);
    assert_eq!(snapshot_ids(&writer), [7]);
}

#[test]
fn a_large_export_reports_progress_as_it_is_read() {
    let (dir, writer) = db();
    let xml = dir.path().join("big.xml");
    // A few MiB, so progress is reported along the way.
    let ids: Vec<u64> = (1..=30_000).collect();
    write_at(&xml, &export(&ids), 5);
    let updates = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let heard = updates.clone();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(move |u: &[jobs::JobUpdate]| heard.lock().unwrap().extend_from_slice(u))
        .handler(JobKind::ReadRekordbox, XmlReader::default())
        .start()
        .unwrap();
    let id = queue.enqueue(read_job(&xml)).unwrap();
    let start = Instant::now();
    while !writer
        .call(move |c| jobs::store::get(c, id))
        .unwrap()
        .unwrap()
        .status
        .is_finished()
    {
        assert!(start.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(5));
    }
    queue.shutdown();
    assert_eq!(snapshot_ids(&writer).len(), 30_000);
    let stored: Vec<f64> = writer
        .call(move |c| jobs::store::get(c, id))
        .map(|j| j.unwrap().progress.into_iter().collect())
        .unwrap();
    assert_eq!(stored, [1.0]);
    // Every update is ours; at least one came between start and done.
    std::thread::sleep(crate::jobs::BATCH_WINDOW * 4);
    let partial = updates
        .lock()
        .unwrap()
        .iter()
        .filter_map(|u| u.progress)
        .filter(|p| *p > 0.0 && *p < 1.0)
        .count();
    assert!(partial > 0, "no progress between start and done");
}

// --- nothing written outside the app data folder ---

/// Every entry under `dir`: bytes and modified time for files, modified
/// time for folders.
fn disk_state(dir: &Path, out: &mut BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            disk_state(&path, out);
            out.insert(path, (None, meta.modified().unwrap()));
        } else {
            out.insert(
                path.clone(),
                (Some(fs::read(&path).unwrap()), meta.modified().unwrap()),
            );
        }
    }
}

#[test]
fn reading_and_watching_leave_the_export_and_its_folder_untouched() {
    let (_data, writer) = db();
    let documents = tempfile::tempdir().unwrap();
    let docs = documents.path();
    let xml = docs.join("rekordbox.xml");
    write_at(&xml, &export(&[1, 2]), 1_000);
    write_at(&docs.join("broken.xml"), "<DJ_PLAYLISTS><COLLECTION", 2_000);
    write_at(&docs.join("notes.txt"), "not an export", 3_000);
    fs::create_dir(docs.join("rekordbox")).unwrap();
    write_at(&docs.join("rekordbox").join("old.xml"), &export(&[9]), 500);
    let mut before = BTreeMap::new();
    disk_state(docs, &mut before);
    let parent_before = fs::metadata(docs).unwrap().modified().unwrap();

    assert_eq!(read_as_job(&writer, &xml).0, JobStatus::Done);
    assert_eq!(
        read_as_job(&writer, &docs.join("broken.xml")).0,
        JobStatus::Failed
    );
    writer.call(|c| write_setting(c, XML_WATCH, &true)).unwrap();
    let folder = docs.to_owned();
    writer.call(move |c| source(c, Some(&folder))).unwrap();

    let mut after = BTreeMap::new();
    disk_state(docs, &mut after);
    assert_eq!(before, after, "the export folder changed");
    assert_eq!(
        fs::metadata(docs).unwrap().modified().unwrap(),
        parent_before
    );
}

// --- the watch ---

fn last_read_at(path: &Path, modified_ms: i64) -> LastRead {
    LastRead {
        path: path.to_string_lossy().into_owned(),
        modified_ms,
        read_at: "2026-09-29T00:00:00.000Z".into(),
        summary: SnapshotSummary {
            tracks: 1,
            streaming: 0,
            kept: 0,
            not_stored: 0,
            complete: true,
        },
    }
}

#[test]
fn the_watch_offers_the_newest_export_modified_after_the_last_read() {
    let docs = tempfile::tempdir().unwrap();
    let docs = docs.path();
    write_at(&docs.join("rekordbox.xml"), &export(&[1]), 1_000);
    write_at(&docs.join("export 2.XML"), &export(&[1, 2]), 3_000);
    write_at(&docs.join("export 1.xml"), &export(&[1, 2]), 2_000);
    let last = last_read_at(&docs.join("rekordbox.xml"), 1_000_000);
    let found = newer_export(exports_in(docs), Some(&last), &NotExports::default()).unwrap();
    assert_eq!(found.name, "export 2.XML");
    assert_eq!(found.path, docs.join("export 2.XML").to_string_lossy());
    assert_eq!(found.modified_ms, 3_000_000);
}

#[test]
fn the_watch_ignores_other_files_older_exports_and_exports_in_subfolders() {
    let docs = tempfile::tempdir().unwrap();
    let docs = docs.path();
    let newer = 9_000;
    // Newer, but not rekordbox exports.
    write_at(
        &docs.join("playlist.xml"),
        r#"<?xml version="1.0"?><playlist/>"#,
        newer,
    );
    write_at(&docs.join("notes.txt"), &export(&[1]), newer);
    write_at(&docs.join("rekordbox.xml.bak"), &export(&[1]), newer);
    fs::create_dir(docs.join("folder.xml")).unwrap();
    // Exports, but not newer, or not directly in the folder.
    write_at(&docs.join("rekordbox.xml"), &export(&[1]), 1_000);
    write_at(&docs.join("older.xml"), &export(&[1]), 500);
    fs::create_dir(docs.join("Backups")).unwrap();
    write_at(&docs.join("Backups").join("new.xml"), &export(&[1]), newer);

    let last = last_read_at(&docs.join("rekordbox.xml"), 1_000_000);
    assert_eq!(
        newer_export(exports_in(docs), Some(&last), &NotExports::default()),
        None
    );
}

#[test]
fn before_any_read_the_watch_offers_the_newest_export_found() {
    let docs = tempfile::tempdir().unwrap();
    let docs = docs.path();
    write_at(&docs.join("a.xml"), &export(&[1]), 1_000);
    write_at(&docs.join("b.xml"), &export(&[1]), 2_000);
    assert_eq!(
        newer_export(exports_in(docs), None, &NotExports::default())
            .unwrap()
            .name,
        "b.xml"
    );
}

#[test]
fn the_chosen_export_saved_again_in_place_is_offered_even_outside_documents() {
    let (_data, writer) = db();
    let docs = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let xml = elsewhere.path().join("collection.xml");
    write_at(&xml, &export(&[1]), 1_000);
    assert_eq!(read_as_job(&writer, &xml).0, JobStatus::Done);
    let chosen = xml.to_string_lossy().into_owned();
    writer
        .call(move |c| {
            write_setting(c, XML_PATH, &chosen)?;
            write_setting(c, XML_WATCH, &true)
        })
        .unwrap();
    let docs_path = docs.path().to_owned();
    let check = move |w: &Writer| {
        let docs = docs_path.clone();
        w.call(move |c| source(c, Some(&docs))).unwrap()
    };
    assert_eq!(check(&writer).newer_export, None, "just read it");

    write_at(&xml, &export(&[1, 2]), 2_000);
    let offered = check(&writer).newer_export.unwrap();
    assert_eq!(offered.path, xml.to_string_lossy());
}

#[test]
fn with_the_watch_off_nothing_is_offered() {
    let (_data, writer) = db();
    let docs = tempfile::tempdir().unwrap();
    write_at(&docs.path().join("rekordbox.xml"), &export(&[1]), 1_000);
    let folder = docs.path().to_owned();
    let source = writer.call(move |c| source(c, Some(&folder))).unwrap();
    assert!(!source.watch);
    assert_eq!(source.newer_export, None);
    assert_eq!(
        source.export_folder,
        Some(docs.path().to_string_lossy().into_owned())
    );
}

#[test]
fn a_stored_setting_this_version_cannot_read_counts_as_unset() {
    let (_data, writer) = db();
    writer
        .call(|c| {
            for key in [XML_PATH, XML_WATCH, LAST_READ, LAST_FAILURE] {
                c.execute(
                    "INSERT INTO setting (key, value) VALUES (?1, '{\"future\": 1}')",
                    [key],
                )?;
            }
            Ok(())
        })
        .unwrap();
    let source = stored(&writer);
    assert_eq!(
        source,
        XmlSource {
            path: None,
            watch: false,
            last_read: None,
            last_failure: None,
            newer_export: None,
            export_folder: None,
        }
    );
}

// --- through the app, as the frontend calls it ---

use crate::ipc::testing::{app, invoke};
use tauri::Manager;

fn wait_for_read(app: &tauri::App<tauri::test::MockRuntime>) -> Value {
    let start = Instant::now();
    loop {
        let source = invoke(app, "rekordbox_xml_source", json!({})).unwrap();
        if !source["lastRead"].is_null() || !source["lastFailure"].is_null() {
            return source;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "never read");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_frontend_can_choose_an_export_read_it_and_read_it_again() {
    let (_data, app) = app();
    let files = tempfile::tempdir().unwrap();
    let xml = files.path().join("rekordbox.xml");
    write_at(&xml, &export(&[1, 2]), 1_000);
    let path = xml.to_string_lossy().into_owned();

    let job = invoke(&app, "read_rekordbox_xml", json!({ "path": path })).unwrap();
    assert!(job.is_number(), "{job}");
    let source = wait_for_read(&app);
    assert_eq!(source["path"], json!(path));
    assert_eq!(source["lastRead"]["path"], json!(path));
    assert_eq!(source["lastRead"]["summary"]["tracks"], json!(2));
    assert_eq!(source["lastRead"]["summary"]["complete"], json!(true));
    assert_eq!(source["watch"], json!(false));

    // Read again, with no path: the chosen export.
    write_at(&xml, &export(&[1, 2, 3]), 2_000);
    invoke(&app, "read_rekordbox_xml", json!({})).unwrap();
    let start = Instant::now();
    loop {
        let source = invoke(&app, "rekordbox_xml_source", json!({})).unwrap();
        if source["lastRead"]["summary"]["tracks"] == json!(3) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "never read again"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn choosing_a_missing_file_or_one_that_is_not_an_export_is_refused_and_keeps_the_chosen_one() {
    let (_data, app) = app();
    let files = tempfile::tempdir().unwrap();
    let good = files.path().join("rekordbox.xml");
    write_at(&good, &export(&[1]), 1_000);
    let other = files.path().join("other.xml");
    write_at(&other, "<playlist/>", 1_000);
    let good_path = good.to_string_lossy().into_owned();
    invoke(&app, "read_rekordbox_xml", json!({ "path": good_path })).unwrap();
    wait_for_read(&app);

    let missing = files.path().join("gone.xml").to_string_lossy().into_owned();
    let other = other.to_string_lossy().into_owned();
    for (path, kind) in [
        (missing, "rekordboxXmlNotFound"),
        (other, "notRekordboxXml"),
        ("relative.xml".to_owned(), "rekordboxXmlNotFound"),
    ] {
        let refused = invoke(&app, "read_rekordbox_xml", json!({ "path": path })).unwrap_err();
        assert_eq!(refused, json!({ "kind": kind, "params": { "path": path } }));
    }
    let source = invoke(&app, "rekordbox_xml_source", json!({})).unwrap();
    assert_eq!(source["path"], json!(good_path));
}

#[test]
fn reading_again_before_any_export_is_chosen_is_refused() {
    let (_data, app) = app();
    let refused = invoke(&app, "read_rekordbox_xml", json!({})).unwrap_err();
    assert_eq!(refused, json!({ "kind": "noRekordboxXml", "params": {} }));
}

#[test]
fn the_frontend_can_turn_the_watch_on_and_hears_of_a_newer_export() {
    let (_data, app) = app();
    let docs = tempfile::tempdir().unwrap();
    app.state::<ExportFolder>()
        .set(Some(docs.path().to_owned()));
    write_at(&docs.path().join("rekordbox.xml"), &export(&[1]), 1_000);

    let off = invoke(&app, "rekordbox_xml_source", json!({})).unwrap();
    assert_eq!(off["newerExport"], Value::Null);
    invoke(&app, "set_rekordbox_xml_watch", json!({ "watch": true })).unwrap();
    let on = invoke(&app, "rekordbox_xml_source", json!({})).unwrap();
    assert_eq!(on["watch"], json!(true));
    assert_eq!(on["newerExport"]["name"], json!("rekordbox.xml"));
    assert_eq!(on["exportFolder"], json!(docs.path().to_string_lossy()));

    invoke(&app, "set_rekordbox_xml_watch", json!({ "watch": false })).unwrap();
    let off = invoke(&app, "rekordbox_xml_source", json!({})).unwrap();
    assert_eq!(
        (off["watch"].clone(), off["newerExport"].clone()),
        (json!(false), Value::Null)
    );
}

#[test]
fn the_app_finds_the_documents_folder_as_the_default_export_folder() {
    let (_data, app) = app();
    let folder = app.state::<ExportFolder>().get();
    assert_eq!(folder, app.path().document_dir().ok());
}

/// Every permission a capability grants: a name, or an object naming it.
fn permission_names(capability: &Value) -> Vec<String> {
    capability["permissions"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|p| {
                    p.as_str()
                        .or_else(|| p["identifier"].as_str())
                        .unwrap_or_else(|| panic!("unreadable permission {p}"))
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_webview_is_granted_the_core_defaults_and_the_file_picker_and_nothing_else() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut granted = Vec::new();
    // Every capability file.
    for entry in fs::read_dir(root.join("capabilities")).unwrap() {
        let path = entry.unwrap().path();
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("json"),
            "a capability this test can't read: {}",
            path.display()
        );
        let capability: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        granted.extend(permission_names(&capability));
    }
    // And any capability written inline in the app's config.
    let config: Value =
        serde_json::from_str(&fs::read_to_string(root.join("tauri.conf.json")).unwrap()).unwrap();
    for capability in config["app"]["security"]["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
    {
        granted.extend(permission_names(capability));
    }
    granted.sort();
    assert_eq!(granted, ["core:default", "dialog:allow-open"]);
}

// --- review follow-ups: cancelling, the read's transaction, online-only files ---

/// A read of `path` whose job is cancelled when it first reaches `stage`.
/// Returns the job's end, and every stage the read reached.
fn cancel_at(writer: &Writer, path: &Path, stage: Stage) -> (JobStatus, Vec<Stage>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};

    let (reached, at) = mpsc::channel::<()>();
    let (go, resume) = mpsc::channel::<()>();
    let (reached, resume) = (Mutex::new(reached), Mutex::new(resume));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fired = AtomicBool::new(false);
    let log = seen.clone();
    let reader = XmlReader {
        hook: Some(Arc::new(move |s: Stage| {
            log.lock().unwrap().push(s);
            if s == stage && !fired.swap(true, Ordering::SeqCst) {
                reached.lock().unwrap().send(()).unwrap();
                // Wait here while the test cancels the job.
                let _ = resume.lock().unwrap().recv_timeout(Duration::from_secs(30));
            }
        })),
    };
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::ReadRekordbox, reader)
        .start()
        .unwrap();
    let id = queue.enqueue(read_job(path)).unwrap();
    at.recv_timeout(Duration::from_secs(60))
        .expect("the read never reached the stage");
    queue.cancel(id).unwrap();
    go.send(()).unwrap();
    let start = Instant::now();
    let status = loop {
        let job = writer
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
    let seen = seen.lock().unwrap().clone();
    (status, seen)
}

/// The snapshot's rows and the stored source: everything a read changes.
type ReadState = (Vec<Vec<Option<String>>>, XmlSource);

/// A database with one export already read, and the state it left.
fn after_one_read() -> (tempfile::TempDir, Writer, ReadState) {
    let (dir, writer) = db();
    let earlier = dir.path().join("earlier.xml");
    write_at(&earlier, &export(&[1]), 5);
    assert_eq!(read_as_job(&writer, &earlier).0, JobStatus::Done);
    let state = (snapshot_rows(&writer), stored(&writer));
    (dir, writer, state)
}

#[test]
fn cancelling_partway_through_reading_stops_the_read_and_changes_nothing() {
    let (dir, writer, before) = after_one_read();
    // A few MiB, so the read reports progress more than once.
    let big = dir.path().join("big.xml");
    write_at(&big, &export(&(1..=30_000).collect::<Vec<_>>()), 6);
    let (status, stages) = cancel_at(&writer, &big, Stage::Progress);
    assert_eq!(status, JobStatus::Cancelled);
    // It stopped at the next progress report instead of reading on.
    let reports = stages.iter().filter(|s| **s == Stage::Progress).count();
    assert_eq!(reports, 1, "{stages:?}");
    assert_eq!((snapshot_rows(&writer), stored(&writer)), before);
    assert_eq!(stored(&writer).last_failure, None);
}

#[test]
fn cancelling_after_the_file_is_parsed_stores_nothing() {
    let (dir, writer, before) = after_one_read();
    let small = dir.path().join("small.xml");
    write_at(&small, &export(&[2, 3]), 6);
    let (status, _) = cancel_at(&writer, &small, Stage::Parsed);
    assert_eq!(status, JobStatus::Cancelled);
    assert_eq!((snapshot_rows(&writer), stored(&writer)), before);
    assert_eq!(stored(&writer).last_failure, None);
}

#[test]
fn if_the_read_cannot_be_recorded_the_snapshot_and_chosen_export_stay_as_they_were() {
    let (dir, writer, before) = after_one_read();
    // Recording the read fails, as a full disk would make it. The setting
    // already exists, so it's an update.
    writer
        .call(|c| {
            c.execute_batch(&format!(
                "CREATE TEMP TRIGGER fail_last_read BEFORE UPDATE ON main.setting
                 WHEN new.key = '{LAST_READ}'
                 BEGIN SELECT RAISE(ABORT, 'disk full'); END;"
            ))
        })
        .unwrap();
    let newer = dir.path().join("newer.xml");
    write_at(&newer, &export(&[2, 3]), 6);
    let (status, _) = read_as_job(&writer, &newer);
    assert_eq!(status, JobStatus::Failed);
    assert_eq!((snapshot_rows(&writer), stored(&writer)), before);
}

/// Marks `path` the way Windows marks a file whose data isn't on this PC.
#[cfg(windows)]
fn make_online_only(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated path that outlives both calls.
    unsafe {
        let attrs = GetFileAttributesW(wide.as_ptr());
        assert_ne!(attrs, u32::MAX);
        assert_ne!(
            SetFileAttributesW(wide.as_ptr(), attrs | FILE_ATTRIBUTE_OFFLINE),
            0
        );
    }
    assert!(
        is_online_only(&fs::metadata(path).unwrap()),
        "the attribute didn't stick"
    );
}

#[cfg(windows)]
#[test]
fn the_watch_never_opens_or_offers_an_online_only_file() {
    use std::sync::Mutex;
    let docs = tempfile::tempdir().unwrap();
    let docs = docs.path();
    write_at(&docs.join("local.xml"), &export(&[1]), 1_000);
    // Newer, and an export, but its data isn't on this PC.
    let cloud = docs.join("cloud.xml");
    write_at(&cloud, &export(&[1, 2]), 2_000);
    make_online_only(&cloud);

    assert!(
        exports_in(docs).iter().all(|c| c.path != cloud),
        "listed an online-only file"
    );
    let opened = Mutex::new(Vec::new());
    let found = newer_export_by(exports_in(docs), None, |c| {
        opened.lock().unwrap().push(c.path.clone());
        looks_like_export(&c.path)
    })
    .unwrap();
    assert_eq!(found.name, "local.xml");
    assert_eq!(opened.into_inner().unwrap(), [docs.join("local.xml")]);
}

#[test]
fn the_chosen_export_is_offered_by_its_modified_time_without_being_opened() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("collection.xml");
    // Its contents don't matter until it's read.
    write_at(&chosen, "still being written", 2_000);
    let last = last_read_at(&chosen, 1_000_000);
    let found = newer_export_by(chosen_candidate(&chosen), Some(&last), |c| {
        panic!("opened {}", c.path.display())
    })
    .unwrap();
    assert_eq!(found.path, chosen.to_string_lossy());
}

#[test]
fn a_picked_file_that_fails_to_read_does_not_replace_the_chosen_export() {
    let (_data, app) = app();
    let files = tempfile::tempdir().unwrap();
    let good = files.path().join("rekordbox.xml");
    write_at(&good, &export(&[1]), 1_000);
    let good_path = good.to_string_lossy().into_owned();
    invoke(&app, "read_rekordbox_xml", json!({ "path": good_path })).unwrap();
    wait_for_read(&app);

    // Starts like an export, so it's accepted, but it's cut off.
    let cut = files.path().join("cut.xml");
    let full = export(&[1, 2]);
    write_at(&cut, &full[..full.len() / 2], 2_000);
    let cut_path = cut.to_string_lossy().into_owned();
    invoke(&app, "read_rekordbox_xml", json!({ "path": cut_path })).unwrap();
    let start = Instant::now();
    let source = loop {
        let source = invoke(&app, "rekordbox_xml_source", json!({})).unwrap();
        if !source["lastFailure"].is_null() {
            break source;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "never failed");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(source["lastFailure"]["path"], json!(cut_path));
    assert_eq!(source["lastFailure"]["reason"], json!("damaged"));
    assert_eq!(source["path"], json!(good_path));
}

#[cfg(windows)]
#[test]
fn the_chosen_export_is_not_offered_once_its_data_is_online_only() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("rekordbox.xml");
    write_at(&chosen, &export(&[1]), 2_000);
    assert!(chosen_candidate(&chosen).is_some());
    make_online_only(&chosen);
    assert!(chosen_candidate(&chosen).is_none());

    let (_data, writer) = db();
    let path = chosen.to_string_lossy().into_owned();
    writer
        .call(move |c| {
            write_setting(c, XML_PATH, &path)?;
            write_setting(c, XML_WATCH, &true)
        })
        .unwrap();
    let source = writer.call(|c| source(c, None)).unwrap();
    assert_eq!(source.newer_export, None);
}

#[test]
fn the_chosen_export_is_found_in_its_folder_listing_whatever_the_letter_case() {
    let dir = tempfile::tempdir().unwrap();
    write_at(&dir.path().join("Rekordbox.XML"), &export(&[1]), 2_000);
    let asked = dir.path().join(if cfg!(windows) {
        "rekordbox.xml"
    } else {
        "Rekordbox.XML"
    });
    let found = chosen_candidate(&asked).unwrap();
    assert_eq!(found.modified_ms, 2_000_000);
    assert!(chosen_candidate(&dir.path().join("gone.xml")).is_none());
}

#[test]
fn a_file_found_not_to_be_an_export_is_opened_once_until_it_changes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let docs = tempfile::tempdir().unwrap();
    let docs = docs.path();
    let notes = docs.join("notes.xml");
    write_at(&notes, "<playlist/>", 1_000);
    let not_exports = NotExports::default();
    let opened = AtomicUsize::new(0);
    let poll = || {
        newer_export_by(exports_in(docs), None, |c| {
            not_exports.check(c, |p| {
                opened.fetch_add(1, Ordering::SeqCst);
                looks_like_export(p)
            })
        })
    };
    // Three polls, as the screen makes every 15 s: opened once.
    for _ in 0..3 {
        assert_eq!(poll(), None);
    }
    assert_eq!(opened.load(Ordering::SeqCst), 1);
    // Saved again, it's a new version: looked at again, and now it's one.
    write_at(&notes, &export(&[1]), 2_000);
    assert_eq!(poll().unwrap().name, "notes.xml");
    assert_eq!(opened.load(Ordering::SeqCst), 2);
}
