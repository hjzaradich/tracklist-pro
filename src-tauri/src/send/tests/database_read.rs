//! A read of rekordbox's database after an export read (Run 3C job D). The
//! database read carries no `Grouping`, `DateModified` or `LastPlayed`, and
//! a `Kind` for five file types only (`rekordbox_db::library`). Only the
//! merge before a send compares rekordbox's values with the bases and the
//! app's own values (`conflicts::merge_before_send`), and a send refuses to
//! start while the newest read is the database's ([`export_to_send`]), so
//! the missing fields never read as rekordbox clearing them. Every value is
//! invented.

use std::sync::Arc;

use super::*;
use crate::conflicts::app_values;
use crate::jobs::{JobContext, JobError};
use crate::rekordbox::source::{stored_source, DatabaseSource, ReadSource, XmlReader};

/// The track as an export gives it, with the fields the database read
/// lacks.
const FROM_EXPORT: &str =
    r#"Grouping="Red" Kind="MP3 File" DateModified="2026-09-20" LastPlayed="2026-09-27""#;

fn collection(extra: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="1">
    <TRACK TrackID="40" Name="Known in rekordbox" Artist="Kit" Genre="House" {extra} Rating="204" PlayCount="7" AverageBpm="128.00" Tonality="Am" Location="file://localhost/E:/Music/known.mp3"/>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="0"/>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#
    )
}

/// Stands in for the database reader: the same track, without the fields
/// the database read doesn't carry.
struct Database;

impl DatabaseSource for Database {
    fn read(&self, _job: &JobContext, _path: &Path) -> Result<(RekordboxXml, i64), JobError> {
        let xml = RekordboxXml::parse(collection("").as_bytes()).unwrap();
        Ok((xml, 4_200_000_000_000))
    }
}

/// What the app holds of its own and what it last sent, and the conflicts.
fn library_values(w: &World) -> (Vec<(i64, String, String)>, Vec<(i64, String, String)>, i64) {
    let rows = |sql: &'static str| {
        w.writer
            .call(move |c| {
                c.prepare(sql)?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
    };
    let conflicts = w
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM conflict", [], |r| r.get(0)))
        .unwrap();
    (
        rows("SELECT library_track_id, field, value FROM app_value ORDER BY 1, 2"),
        rows("SELECT library_track_id, field, value FROM sync_base ORDER BY 1, 2"),
        conflicts,
    )
}

fn read_the_database(w: &World) {
    let queue = JobQueue::builder(w.writer.clone())
        .workers(1)
        .handler(
            JobKind::ReadRekordbox,
            XmlReader::default().with_database(Arc::new(Database)),
        )
        .start()
        .unwrap();
    let job = NewJob::new(JobKind::ReadRekordbox)
        .target(json!({ "path": "C:/rb/master.db", "source": "database" }));
    let id = queue.enqueue(job).unwrap();
    let start = Instant::now();
    loop {
        let job = w
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            queue.shutdown();
            assert_eq!(job.status, JobStatus::Done, "{:?}", job.error);
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the read never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_database_read_missing_fields_an_export_had_opens_no_conflict_and_changes_no_library_value() {
    let w = World::new();
    let (known, _) = w.library_track("known.mp3", "Known", true);
    w.crate_of("Warm up", &[known]);
    w.save_export_text(&collection(FROM_EXPORT));
    assert!(w.prepare().is_some());
    assert_eq!(w.go(true), None);
    // The user's own value, edited after the send.
    w.writer
        .call(move |c| Ok(app_values::set(c, &Plugged, known, "Genre", "Deep House")))
        .unwrap()
        .unwrap();
    let before = library_values(&w);
    let sent: Vec<&str> = before
        .1
        .iter()
        .map(|(_, field, _)| field.as_str())
        .collect();
    for field in ["Grouping", "Kind", "DateModified", "LastPlayed"] {
        assert!(sent.contains(&field), "{field} was sent and has a base");
    }

    read_the_database(&w);

    let stored = w.writer.call(|c| stored_source(c)).unwrap();
    assert_eq!(
        stored.last_read.as_ref().unwrap().source,
        ReadSource::Database
    );
    assert_eq!(library_values(&w), before);
    assert_eq!(before.2, 0, "no conflict before or after");
    // The only place the fields are compared, the merge before a send,
    // isn't reached: the send refuses to start from this read.
    let refused = export_to_send(&stored, None).unwrap_err();
    assert_eq!(refused.kind(), crate::ipc::ErrorKind::SendNeedsExport);
}
