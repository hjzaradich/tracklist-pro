//! The send review's warning about files changed after the export (1cB-4;
//! ROADMAP 1.10, "Second channel"): counted from the modified time the
//! scan recorded, a warning only, and never a source of rekordbox's
//! values. Times are set by hand; every name is invented.

use super::*;
use crate::send::preflight::changed_after_export;

/// One millisecond, in the nanoseconds `file.mtime` holds.
const MS: i64 = 1_000_000;

impl World {
    /// When rekordbox saved the export last, as [`ExportRead::modified_ms`]
    /// gives it.
    fn export_ms(&self) -> i64 {
        i64::try_from(self.saved.get()).unwrap() * 1000
    }

    /// The scan recorded this modified time for `file`, in nanoseconds
    /// since the Unix epoch.
    fn file_modified(&self, file: i64, mtime_ns: i64) {
        self.insert("UPDATE file SET mtime = ?1 WHERE id = ?2", (mtime_ns, file));
    }

    fn linked_file(&self, track: LibraryTrackId) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn count_of(&self, table: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
            .unwrap()
    }
}

fn listed(preflight: &Preflight) -> Vec<LibraryTrackId> {
    preflight
        .changed_after_export
        .iter()
        .map(|t| t.library_track)
        .collect()
}

#[test]
fn a_track_rekordbox_has_whose_file_changed_after_the_export_is_counted_and_listed() {
    let (w, known, _) = world_with_a_send_waiting();
    // One millisecond after the export was saved.
    w.file_modified(w.linked_file(known), (w.export_ms() + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(listed(&preflight), [known]);
    assert_eq!(
        preflight.changed_after_export[0].title.as_deref(),
        Some("Known")
    );
}

#[test]
fn a_file_changed_before_the_export_or_unchanged_since_is_not_counted() {
    let w = World::new();
    let (before, before_file) = w.library_track("before.mp3", "Before", true);
    let (same_ms, same_ms_file) = w.library_track("same.mp3", "Same", true);
    let (never_seen, _) = w.library_track("unstated.mp3", "Unstated", true);
    let (after, after_file) = w.library_track("after.mp3", "After", true);
    w.crate_of("Warm up", &[before, same_ms, never_seen, after]);
    w.save_export(&[
        Rb(40, "before.mp3", "Before"),
        Rb(41, "same.mp3", "Same"),
        Rb(42, "unstated.mp3", "Unstated"),
        Rb(43, "after.mp3", "After"),
    ]);
    let export = w.export_ms();
    // A minute before the export; within the export's own millisecond;
    // no modified time recorded at all; and the first nanosecond after it.
    w.file_modified(before_file, (export - 60_000) * MS);
    w.file_modified(same_ms_file, export * MS + MS - 1);
    w.file_modified(after_file, (export + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(preflight.known_tracks, 4);
    assert_eq!(listed(&preflight), [after]);
}

#[test]
fn a_file_no_library_track_links_a_track_the_send_leaves_out_and_a_new_track_are_not_counted() {
    let w = World::new();
    let (sent, _) = w.library_track("sent.mp3", "Sent", true);
    // rekordbox has it, but no crate names it: it isn't sent.
    let (_, loose_file) = w.library_track("loose.mp3", "Loose", true);
    // rekordbox doesn't have it: its values come from its own file.
    let (new, new_file) = w.library_track("new.mp3", "New", true);
    // In All music and in rekordbox, but in no Library track.
    let recording = w.insert("INSERT INTO recording (title) VALUES ('Stray')", ());
    let stray_file = w.file(recording, "stray.mp3", "Stray", true);
    w.crate_of("Warm up", &[sent, new]);
    w.save_export(&[
        Rb(40, "sent.mp3", "Sent"),
        Rb(41, "loose.mp3", "Loose"),
        Rb(42, "stray.mp3", "Stray"),
    ]);
    let after = (w.export_ms() + 1) * MS;
    for file in [loose_file, new_file, stray_file] {
        w.file_modified(file, after);
    }

    let preflight = w.prepare().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert!(listed(&preflight).is_empty());
}

#[test]
fn a_track_is_counted_by_the_file_rekordboxs_entry_is_matched_to_as_well() {
    let w = World::new();
    let (known, file) = w.library_track("known.mp3", "Known", true);
    let (other, _) = w.library_track("other.mp3", "Other", true);
    w.crate_of("Warm up", &[known, other]);
    w.save_export(&[Rb(40, "known.mp3", "Known")]);
    let export = w.export_ms();
    w.file_modified(file, (export + 1) * MS);
    w.prepare().unwrap();

    // `other`'s own file is unchanged, but it goes out as rekordbox's entry
    // 40 (as a copy's entry does), whose file changed.
    let sent_as_40 = |track: LibraryTrackId, in_rekordbox: bool| SentTrack {
        library_track: track,
        in_rekordbox,
        track_id: 40,
        attributes: Vec::new(),
        rekordbox_holds_other_file: None,
        file_missing: false,
    };
    let sent = [sent_as_40(other, true)];
    let found = w
        .writer
        .call(move |c| changed_after_export(c, export, &sent))
        .unwrap();
    assert_eq!(found, [other]);
    // A new track numbered 40 in the file isn't rekordbox's entry 40.
    let sent = [sent_as_40(other, false)];
    let found = w
        .writer
        .call(move |c| changed_after_export(c, export, &sent))
        .unwrap();
    assert!(found.is_empty());
}

#[test]
fn the_send_still_goes_with_the_warning_and_needs_no_confirm() {
    let (w, known, _) = world_with_a_send_waiting();
    w.file_modified(w.linked_file(known), (w.export_ms() + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(listed(&preflight), [known]);
    assert!(preflight.can_send && !preflight.needs_confirm);
    assert_eq!(preflight.refusal, None);

    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn a_file_tag_is_never_taken_as_rekordboxs_value_and_the_warning_changes_nothing_sent() {
    let w = World::new();
    let (known, file) = w.library_track("known.mp3", "Known", true);
    w.crate_of("Warm up", &[known]);
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    // rekordbox wrote an edit into the file after the export: the scan
    // read a new title and a new modified time.
    let tags = json!({ "id3v2": [{"key": "TIT2", "value": {"type": "text", "text": "Retitled in the file"}}] })
        .to_string();
    w.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (tags, file));
    w.file_modified(file, (w.export_ms() + 30_000) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(listed(&preflight), [known]);
    assert_eq!(w.go(false), None);
    // rekordbox's value went out, not the file's; the merge opened no
    // conflict and took nothing in as the app's own value.
    assert_eq!(w.sent_titles(), ["Known in rekordbox"]);
    assert_eq!((w.count_of("conflict"), w.count_of("app_value")), (0, 0));
    let warned = fs::read(w.send_file()).unwrap();

    // The same export saved again, after the file changed: no warning, and
    // the very same bytes go out.
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    let preflight = w.prepare().unwrap();
    assert!(listed(&preflight).is_empty());
    assert_eq!(w.go(false), None);
    assert_eq!(fs::read(w.send_file()).unwrap(), warned);
    assert_eq!((w.count_of("conflict"), w.count_of("app_value")), (0, 0));
}

/// Builds a Library of `count` tracks, every one in rekordbox and every
/// tenth file changed after the export, and times the count over a send
/// of them all.
#[test]
#[ignore = "builds 100,000 tracks; run with -- --ignored --nocapture"]
fn counting_over_100000_tracks_is_cheap() {
    let count: i64 = 100_000;
    let w = World::new();
    let export: i64 = 4_102_444_860_000;
    w.writer
        .call(move |c| {
            c.execute_batch(&format!(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                 INSERT INTO recording (id) SELECT i FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                 INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, present, mtime)
                 SELECT i, 1, 'f' || i || '.mp3', 'f' || i || '.mp3', 1,
                        CASE WHEN i % 10 = 0 THEN ({export} + 1) * {MS} ELSE ({export} - 1000) * {MS} END
                 FROM n;
                 INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                 SELECT id, 'linked', id, 'ok' FROM file ORDER BY id;
                 INSERT INTO rekordbox_track (attributes, location_key, file_id, relink_method, read_at)
                 SELECT json_object('TrackID', CAST(id AS TEXT), 'Location', 'file://localhost/E:/Music/' || rel_path), 'e:/music/' || rel_path_key, id,
                        'path', '2100-01-01T00:00:00.000Z'
                 FROM file;"
            ))
        })
        .unwrap();
    let sent: Vec<SentTrack> = (1..=count)
        .map(|i| SentTrack {
            library_track: LibraryTrackId(i),
            in_rekordbox: true,
            track_id: u64::try_from(i).unwrap(),
            attributes: Vec::new(),
            rekordbox_holds_other_file: None,
            file_missing: false,
        })
        .collect();
    let started = Instant::now();
    let found = w
        .writer
        .call(move |c| changed_after_export(c, export, &sent))
        .unwrap();
    println!(
        "counted {} of {count} tracks in {:?}",
        found.len(),
        started.elapsed()
    );
    assert_eq!(i64::try_from(found.len()).unwrap(), count / 10);
}
