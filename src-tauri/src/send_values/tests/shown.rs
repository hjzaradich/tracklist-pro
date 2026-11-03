//! Which rekordbox entry a track shows (1cD-X2, audit C12): every place that
//! picks one entry for a track gives the send's answer, on a varied library.
//!
//! A Library track's authority is the send ([`sent_as`]). A track outside
//! the Library has no send, so its authority is what it would be sent as
//! once added (linked to the file [`crate::library::linked_file_for`]
//! picks): adding a track never changes the entry it shows.

use super::*;

/// A read before [`READ`], whose rows an incomplete export kept.
const OLDER_READ: &str = "2026-09-01T10:00:00.000Z";

/// One track of the library, and what the send says it is.
struct Case {
    what: &'static str,
    track: i64,
    library: Option<LibraryTrackId>,
}

impl Lib {
    /// rekordbox's entry at `E:\Music\<name>`, matched to `file`. Its BPM
    /// is its own and its genre is `G<bpm>`, so either names the entry.
    fn entry(
        &self,
        name: &str,
        file: Option<i64>,
        probable: bool,
        read_at: &str,
        plays: i64,
        bpm: i64,
    ) -> i64 {
        let (bpm, genre, plays) = (bpm.to_string(), format!("G{bpm}"), plays.to_string());
        self.rekordbox_at(
            name,
            file,
            probable,
            read_at,
            &[
                ("AverageBpm", bpm.as_str()),
                ("Genre", genre.as_str()),
                ("PlayCount", plays.as_str()),
            ],
        )
    }

    fn row_where(&self, sql: &'static str, value: String) -> Option<i64> {
        self.writer
            .call(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(sql, [value], |r| r.get(0)).optional()
            })
            .unwrap()
    }

    /// The entry each place picks for `case`, by the place's name.
    fn picks(&self, case: &Case) -> Vec<(&'static str, Option<i64>)> {
        let track = case.track;
        let mut out = Vec::new();
        // The table's BPM and key: the track's rekordbox `analysis` row.
        let bpm: Option<f64> = self
            .writer
            .call(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT bpm FROM analysis WHERE source = 'rekordbox' AND recording_id = ?1",
                    [track],
                    |r| r.get(0),
                )
                .optional()
            })
            .unwrap()
            .flatten();
        out.push((
            "attach (table BPM, key)",
            bpm.and_then(|b| self.row_where("SELECT id FROM rekordbox_track WHERE bpm = CAST(?1 AS REAL)", b.to_string())),
        ));
        // The track's rekordbox panel: the entry it says the analysis is from.
        let source = self
            .writer
            .call(move |c| crate::attach::for_recording(c, track))
            .unwrap()
            .into_iter()
            .find(|e| e.is_analysis_source);
        out.push((
            "for_recording",
            source.and_then(|e| {
                self.row_where("SELECT id FROM rekordbox_track WHERE track_id = ?1", e.track_id.to_string())
            }),
        ));
        match case.library {
            Some(id) => {
                let genre = self
                    .writer
                    .call(move |c| crate::overview::genres_of(c, &[id]))
                    .unwrap()
                    .remove(&id.0);
                out.push((
                    "Overview genre",
                    genre.and_then(|g| {
                        self.row_where(
                            "SELECT id FROM rekordbox_track WHERE json_extract(attributes, '$.Genre') = ?1",
                            g,
                        )
                    }),
                ));
            }
            None => {
                let removals = self
                    .writer
                    .call(|c| crate::after_send::manual_removals(c, &plugged()))
                    .unwrap();
                let named = removals
                    .into_iter()
                    .find(|r| r.recording_id == track)
                    .and_then(|r| r.path);
                out.push((
                    "removal list",
                    named.and_then(|p| {
                        let name = p.rsplit(['\\', '/']).next().unwrap().to_owned();
                        self.row_where(
                            "SELECT id FROM rekordbox_track WHERE location = ?1",
                            format!("file://localhost/E:/Music/{name}"),
                        )
                    }),
                ));
            }
        }
        out
    }

    /// The send's answer for `case`: as it is linked, or for a track outside
    /// the Library, as it would be linked once added (nothing is written).
    fn sent(&self, case: &Case) -> Option<i64> {
        let (track, library) = (case.track, case.library);
        let sent = self
            .writer
            .call(move |c| {
                let tx = c.transaction()?;
                let id = match library {
                    Some(id) => id,
                    None => {
                        let file = crate::library::linked_file_for(&tx, track)?.map(|f| f.file_id);
                        tx.execute(
                            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                             VALUES (?1, 'linked', ?2, 'ok')",
                            (track, file),
                        )?;
                        LibraryTrackId(tx.last_insert_rowid())
                    }
                };
                let sent = sent_as(&tx, &plugged(), &[(id, None)])?;
                // Dropped, so rolled back: an add is only asked about.
                drop(tx);
                Ok(sent)
            })
            .unwrap();
        match sent[..] {
            [SentAs::Entry(row)] => Some(row),
            _ => None,
        }
    }
}

/// A varied library, grouped and attached as the jobs leave it.
fn varied_library() -> (Lib, Vec<Case>, Vec<(&'static str, Option<i64>)>) {
    let lib = Lib::new();
    let mut cases = Vec::new();
    let mut expected = Vec::new();

    // Two trusted entries for one track: the best file's, and a busier one
    // on the linked file.
    let track = lib.track();
    let linked = lib.file(track, "two-linked.mp3", "undecided", Spec::default());
    let best = lib.file(track, "two-best.mp3", "best", Spec::default());
    lib.entry("two-best.mp3", Some(best), false, READ, 0, 120);
    let busier = lib.entry("two-linked.mp3", Some(linked), false, READ, 50, 128);
    let library = lib.library(track, Some(linked));
    cases.push(Case { what: "two entries", track, library: Some(library) });
    expected.push(("two entries", Some(busier)));

    // One trusted entry and a probable one, more played, on the same file.
    let (library, file) = lib.linked("probable.mp3", Spec::default());
    let trusted = lib.entry("probable.mp3", Some(file), false, READ, 1, 121);
    lib.entry("probable-dup.mp3", Some(file), true, READ, 99, 122);
    cases.push(Case { what: "trusted and probable", track: file_track(&lib, file), library: Some(library) });
    expected.push(("trusted and probable", Some(trusted)));

    // An entry kept from an older read beside one of the latest read.
    let (library, file) = lib.linked("older.mp3", Spec::default());
    let kept = lib.entry("older-kept.mp3", Some(file), false, OLDER_READ, 3, 123);
    lib.entry("older.mp3", Some(file), false, READ, 0, 124);
    cases.push(Case { what: "older read", track: file_track(&lib, file), library: Some(library) });
    expected.push(("older read", Some(kept)));

    // A moved file, as the relink job leaves it: the old file is gone,
    // rekordbox still has the old Location, relink matched it to the new
    // file and the track followed its file there.
    let track = lib.track();
    lib.file(
        track,
        "moved-old.mp3",
        "undecided",
        Spec { present: Some(false), ..Spec::default() },
    );
    let new = lib.file(track, "moved-new.mp3", "best", Spec::default());
    let moved = lib.entry("moved-old.mp3", Some(new), false, READ, 0, 125);
    let library = lib.library(track, Some(new));
    cases.push(Case { what: "moved file", track, library: Some(library) });
    expected.push(("moved file", Some(moved)));

    // A track outside the Library, removed from it before it was ever sent:
    // two entries, the one on its best file more played, the other with the
    // lower TrackID.
    let track = lib.track();
    let other = lib.file(track, "outside-other.mp3", "undecided", Spec::default());
    let best = lib.file(track, "outside-best.mp3", "best", Spec::default());
    lib.entry("outside-other.mp3", Some(other), false, READ, 0, 131);
    let on_best = lib.entry("outside-best.mp3", Some(best), false, READ, 50, 130);
    lib.insert("INSERT INTO library_removal (recording_id) VALUES (?1)", [track]);
    cases.push(Case { what: "outside the Library", track, library: None });
    expected.push(("outside the Library", Some(on_best)));

    // A Library track rekordbox has no entry for.
    let (library, file) = lib.linked("none.mp3", Spec::default());
    cases.push(Case { what: "no entry", track: file_track(&lib, file), library: Some(library) });
    expected.push(("no entry", None));

    // A Library track whose only entry is on another file of its track: the
    // send makes a new track of the linked file.
    let track = lib.track();
    let linked = lib.file(track, "elsewhere.mp3", "undecided", Spec::default());
    let copy = lib.file(track, "elsewhere-copy.mp3", "best", Spec::default());
    lib.entry("elsewhere-copy.mp3", Some(copy), false, READ, 0, 127);
    let library = lib.library(track, Some(linked));
    cases.push(Case { what: "entry on another file", track, library: Some(library) });
    expected.push(("entry on another file", None));

    // Grouping gives each matched entry its file's track; then attach runs.
    lib.writer
        .call(|c| {
            c.execute(
                "UPDATE rekordbox_track SET recording_id =
                     (SELECT recording_id FROM recording_file WHERE file_id = rekordbox_track.file_id)",
                [],
            )?;
            crate::attach::attach(c)
        })
        .unwrap();
    (lib, cases, expected)
}

fn file_track(lib: &Lib, file: i64) -> i64 {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT recording_id FROM recording_file WHERE file_id = ?1",
                [file],
                |r| r.get(0),
            )
        })
        .unwrap()
}

#[test]
fn the_send_says_which_entry_each_track_of_a_varied_library_is() {
    let (lib, cases, expected) = varied_library();
    let sent: Vec<_> = cases.iter().map(|c| (c.what, lib.sent(c))).collect();
    assert_eq!(sent, expected);
}

#[test]
fn every_place_that_picks_a_tracks_rekordbox_entry_picks_the_one_the_send_uses() {
    let (lib, cases, _) = varied_library();
    let mut disagree = Vec::new();
    for case in &cases {
        let sent = lib.sent(case);
        for (place, picked) in lib.picks(case) {
            if picked != sent {
                disagree.push(format!("{}: {place} picks {picked:?}, the send {sent:?}", case.what));
            }
        }
    }
    assert!(disagree.is_empty(), "{}", disagree.join("\n"));
}
