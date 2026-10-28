//! Crate membership merged before every send (1cB-3; ROADMAP 1.10, 1.9
//! rule 6), end to end: rekordbox's export read with its playlists, the
//! merge, the file written and the send recorded. Every name is invented.

use super::*;
use crate::conflicts::{self, membership, Choice, Conflict, Filter};
use crate::crates::{self, CrateId};
use crate::ops::{self, UndoOutcome, UndoRefusal};
use crate::rekordbox::EntryTarget;

/// rekordbox's tracks: `TrackID` and file. `x.mp3` is in no Library.
const TRACKS: [(u32, &str); 4] = [(1, "a.mp3"), (2, "b.mp3"), (3, "c.mp3"), (9, "x.mp3")];

/// An export holding the tracks of [`TRACKS`] whose ids are in `tracks`,
/// saying it holds `entries`, with these playlists (entries by `TrackID`)
/// in the folder `Crates`.
fn export_of(tracks: &[u32], entries: usize, crates: &[(&str, &[u32])]) -> String {
    let tracks: String = TRACKS
        .iter()
        .filter(|(id, _)| tracks.contains(id))
        .map(|(id, file)| {
            format!(
                r#"<TRACK TrackID="{id}" Name="{file}" Artist="Kit" Location="file://localhost/E:/Music/{file}"/>"#
            )
        })
        .collect();
    let playlists: String = crates
        .iter()
        .map(|(name, ids)| {
            let keys: String = ids
                .iter()
                .map(|id| format!(r#"<TRACK Key="{id}"/>"#))
                .collect();
            format!(
                r#"<NODE Name="{name}" Type="1" KeyType="0" Entries="{}">{keys}</NODE>"#,
                ids.len()
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="{entries}">{tracks}</COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Type="0" Name="Crates" Count="{}">{playlists}</NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#,
        crates.len()
    )
}

impl World {
    /// rekordbox saves its export with every track and these playlists.
    fn rekordbox_shows(&self, crates: &[(&str, &[u32])]) {
        self.save_export_text(&export_of(&[1, 2, 3, 9], 4, crates));
    }

    /// A whole send, confirmed if it must be.
    fn send_through(&self) -> Preflight {
        let preflight = self.prepare().unwrap();
        assert!(preflight.can_send, "{preflight:?}");
        assert_eq!(self.go(preflight.needs_confirm), None);
        preflight
    }

    /// The files of the tracks the last send wrote into `Crates/<name>`, in
    /// order.
    fn written(&self, name: &str) -> Vec<String> {
        let read = RekordboxXml::parse(fs::read(self.send_file()).unwrap().as_slice()).unwrap();
        let (_, playlist) = read
            .playlists
            .playlists()
            .into_iter()
            .find(|(folders, p)| folders == &["Crates"] && p.name == name)
            .unwrap_or_else(|| panic!("{name} wasn't sent"));
        playlist
            .entries
            .iter()
            .map(|entry| match entry.target {
                EntryTarget::Track(i) => {
                    let location = read.tracks[i].attrs.get("Location").unwrap();
                    location.rsplit('/').next().unwrap().to_owned()
                }
                _ => panic!("an entry for no track"),
            })
            .collect()
    }

    fn rows<T: Send + 'static>(
        &self,
        sql: &'static str,
        params: impl rusqlite::Params + Send + 'static,
        map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T> + Send + 'static,
    ) -> Vec<T> {
        self.writer
            .call(move |c| c.prepare(sql)?.query_map(params, map)?.collect())
            .unwrap()
    }

    /// The Library tracks a crate holds, by id.
    fn holds(&self, crate_id: i64) -> Vec<i64> {
        self.rows(
            "SELECT library_track_id FROM crate_entry WHERE crate_id = ?1 ORDER BY library_track_id",
            (crate_id,),
            |r| r.get(0),
        )
    }

    /// The base: the Library tracks the last send put in `Crates/<name>`.
    fn base_of(&self, name: &str) -> Vec<i64> {
        let key = format!(r#"["crates","{}"]"#, name.to_lowercase());
        let ids = self
            .writer
            .call(|c| {
                let ids: Vec<LibraryTrackId> = c
                    .prepare("SELECT id FROM library_track")?
                    .query_map([], |r| Ok(LibraryTrackId(r.get(0)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                crate::rekordbox_write::sent_entries(c, &ids)
            })
            .unwrap();
        ids.into_iter()
            .filter(|e| serde_json::to_string(&e.key).unwrap() == key)
            .map(|e| e.library_track.0)
            .collect()
    }

    fn operation_count(&self) -> i64 {
        self.rows("SELECT count(*) FROM operation", (), |r| r.get(0))[0]
    }

    fn open_ones(&self) -> Vec<Conflict> {
        self.writer
            .call(|c| conflicts::open_conflicts(c, &Filter::default()))
            .unwrap()
    }

    fn crate_op<T: Send + 'static>(
        &self,
        op: impl FnOnce(&mut rusqlite::Connection) -> Result<T, ops::OpsError> + Send + 'static,
    ) -> T {
        self.writer.call(move |c| Ok(op(c))).unwrap().unwrap()
    }
}

/// Library tracks `a.mp3`, `b.mp3` and `c.mp3`, all three in rekordbox, and
/// the crate Peak holding a and b, sent and imported: rekordbox's Peak
/// holds a and b, the base is a and b.
fn peak_sent_and_imported() -> (World, [LibraryTrackId; 3], i64) {
    let w = World::new();
    let ids = ["a.mp3", "b.mp3", "c.mp3"].map(|f| w.library_track(f, f, true).0);
    let peak = w.crate_of("Peak", &ids[..2]);
    w.rekordbox_shows(&[]);
    w.send_through();
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    assert_eq!(w.base_of("Peak"), [ids[0].0, ids[1].0]);
    (w, ids, peak)
}

#[test]
fn only_the_apps_adds_and_removals_are_written_and_become_the_base() {
    let (w, [a, b, c], peak) = peak_sent_and_imported();
    w.crate_op(move |conn| crates::remove_on(conn, CrateId(peak), &[b]))
        .unwrap();
    w.crate_op(move |conn| crates::add_on(conn, CrateId(peak), &[c]))
        .unwrap();

    w.send_through();

    assert_eq!(w.written("Peak"), ["a.mp3", "c.mp3"]);
    assert_eq!(w.holds(peak), [a.0, c.0]);
    assert_eq!(w.base_of("Peak"), [a.0, c.0]);
    assert_eq!(w.open_ones(), []);
}

#[test]
fn a_track_the_dj_adds_or_removes_in_rekordbox_is_taken_into_the_crate_outside_the_log() {
    let (w, [a, _, c], peak) = peak_sent_and_imported();
    w.rekordbox_shows(&[("Peak", &[1, 3])]);
    let operations = w.operation_count();

    w.send_through();

    assert_eq!(w.holds(peak), [a.0, c.0]);
    assert_eq!(w.written("Peak"), ["a.mp3", "c.mp3"]);
    assert_eq!(w.base_of("Peak"), [a.0, c.0]);
    assert_eq!(w.operation_count(), operations);
    // Sent and imported, the next send takes nothing in again.
    w.rekordbox_shows(&[("Peak", &[1, 3])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, c.0]);
}

#[test]
fn a_send_never_imported_is_sent_again_not_taken_back_and_forgotten_once_imported() {
    let (w, [a, b, c], peak) = peak_sent_and_imported();
    w.crate_op(move |conn| crates::add_on(conn, CrateId(peak), &[c]))
        .unwrap();
    w.send_through();
    let replaced = || {
        w.rows(
            "SELECT replaced FROM sent_playlist WHERE kind = 'playlist'",
            (),
            |r| r.get::<_, Option<String>>(0),
        )
    };
    assert_eq!(replaced(), [Some(format!("[{}]", c.0))]);

    // Not imported: rekordbox still shows Peak as it was.
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, b.0, c.0]);
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3", "c.mp3"]);
    assert_eq!(w.open_ones(), []);

    // Imported: the read forgets what the send replaced.
    w.rekordbox_shows(&[("Peak", &[1, 2, 3])]);
    w.prepare().unwrap();
    assert_eq!(replaced(), [None]);
}

/// [`peak_sent_and_imported`] as a send from before migration 0036 left it
/// (no entries recorded, so no base), with c added in the app and rekordbox
/// showing a and b: b and c differ on the two sides.
fn peak_with_no_base_and_both_sides_different() -> (World, [LibraryTrackId; 3], i64) {
    let (w, ids, peak) = peak_sent_and_imported();
    w.writer
        .call(|c| {
            c.execute_batch("UPDATE sent_playlist SET send_id = NULL; DELETE FROM sent_entry;")
        })
        .unwrap();
    let c = ids[2];
    w.crate_op(move |conn| crates::remove_on(conn, CrateId(peak), &[ids[1]]))
        .unwrap();
    w.crate_op(move |conn| crates::add_on(conn, CrateId(peak), &[c]))
        .unwrap();
    (w, ids, peak)
}

#[test]
fn both_sides_differing_open_a_conflict_send_rekordboxs_membership_and_keep_the_base() {
    let (w, [a, b, c], peak) = peak_with_no_base_and_both_sides_different();

    let preflight = w.send_through();

    assert_eq!(preflight.conflicts_waiting, 2);
    let open = w.open_ones();
    let field = membership::field_of(&["crates".to_owned(), "peak".to_owned()]);
    let seen: Vec<_> = open
        .iter()
        .map(|o| {
            (
                o.library_track,
                o.field.as_str(),
                o.app_value.as_deref(),
                o.rekordbox_value.as_deref(),
                o.base_value.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (b, field.as_str(), Some("out"), Some("in"), None),
            (c, field.as_str(), Some("in"), Some("out"), None),
        ]
    );
    // rekordbox's membership went out; the crate is the app's.
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3"]);
    assert_eq!(w.holds(peak), [a.0, c.0]);
    // Still open, the same conflicts, after the next read and send.
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    w.send_through();
    assert_eq!(w.open_ones(), open);
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3"]);
}

#[test]
fn a_settled_membership_sets_the_crate_and_goes_out_and_its_undo_is_refused_once_sent() {
    let (w, [a, b, _], peak) = peak_with_no_base_and_both_sides_different();
    w.send_through();
    let open = w.open_ones();
    let (on_b, on_c) = (open[0].id, open[1].id);
    // A membership is "in" or "out", nothing else.
    assert!(matches!(
        w.writer
            .call(move |conn| Ok(conflicts::resolve(
                conn,
                on_c,
                &Choice::Edit("maybe".into())
            )))
            .unwrap(),
        Err(conflicts::ResolveError::BadValue { .. })
    ));
    // Through the Conflicts screen's own path: resolve, which adopts.
    w.writer
        .call(move |conn| Ok(conflicts::resolve(conn, on_b, &Choice::KeepRekordbox)))
        .unwrap()
        .unwrap();
    w.writer
        .call(move |conn| Ok(conflicts::resolve(conn, on_c, &Choice::Edit("out".into()))))
        .unwrap()
        .unwrap();
    assert_eq!(w.holds(peak), [a.0, b.0]);

    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    w.send_through();

    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3"]);
    assert_eq!(w.open_ones(), []);
    assert_eq!(w.base_of("Peak"), [a.0, b.0]);
    match ops::undo_last_via(&w.writer).unwrap() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::SentSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
}

#[test]
fn an_untrusted_or_older_read_takes_nothing_in() {
    let (w, [a, b, _], peak) = peak_sent_and_imported();
    // An incomplete export: b's row is kept from the read before, and
    // rekordbox's Peak no longer shows b. Such a send is refused.
    w.save_export_text(&export_of(&[1, 3, 9], 4, &[("Peak", &[1])]));
    let preflight = w.prepare().unwrap();
    assert!(!preflight.can_send);
    assert_eq!(w.holds(peak), [a.0, b.0]);

    // An export saved before the last send: it can't show that send.
    w.rekordbox_shows(&[("Peak", &[1])]);
    w.export_saved_at(Duration::from_secs(1_000_000_000));
    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    assert_eq!(w.holds(peak), [a.0, b.0]);
}

#[test]
fn a_playlist_the_dj_made_in_rekordbox_is_never_compared_or_taken_in() {
    let w = World::new();
    let ids = ["a.mp3", "b.mp3", "c.mp3"].map(|f| w.library_track(f, f, true).0);
    // Warmup is the DJ's own; the app's crate of that name was never sent.
    let warmup = w.crate_of("Warmup", &ids[..1]);
    // It holds x.mp3 too, which isn't in the Library: the import drops it,
    // so the review counts it and the send needs the confirm.
    w.rekordbox_shows(&[("Warmup", &[1, 2, 3, 9])]);

    let preflight = w.send_through();

    assert_eq!(
        preflight.loses_entries,
        [LosesEntries {
            kind: TreeKind::Crate,
            path: vec!["Warmup".to_owned()],
            lost: 1,
            entries: 2,
        }]
    );
    assert!(preflight.needs_confirm);
    assert_eq!(w.holds(warmup), [ids[0].0]);
    assert_eq!(w.written("Warmup"), ["a.mp3"]);
    assert_eq!(w.open_ones(), []);
}

#[test]
fn a_playlist_renamed_or_deleted_in_rekordbox_takes_nothing_in() {
    let (w, [a, b, _], peak) = peak_sent_and_imported();
    // Renamed in rekordbox, with c added: Peak isn't there any more.
    w.rekordbox_shows(&[("Peak 2", &[1, 2, 3])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, b.0]);
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3"]);

    // Deleted in rekordbox.
    w.rekordbox_shows(&[]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, b.0]);
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3"]);
}

#[test]
fn a_crate_renamed_or_deleted_in_the_app_compares_nothing_and_its_open_conflicts_go() {
    let (w, [a, _, c], peak) = peak_with_no_base_and_both_sides_different();
    w.send_through();
    assert_eq!(w.open_ones().len(), 2);
    w.crate_op(move |conn| crates::rename_on(conn, CrateId(peak), "Prime"))
        .unwrap();
    // The DJ added c to the old playlist: the renamed crate has no base.
    w.rekordbox_shows(&[("Peak", &[1, 2, 3])]);

    w.send_through();

    assert_eq!(w.open_ones(), []);
    assert_eq!(w.holds(peak), [a.0, c.0]);
    assert_eq!(w.written("Prime"), ["a.mp3", "c.mp3"]);

    w.crate_op(move |conn| crates::delete_on(conn, CrateId(peak)))
        .unwrap();
    w.rekordbox_shows(&[("Prime", &[1, 3]), ("Peak", &[1, 2, 3])]);
    // Nothing is left to send; the merge still ran, and took nothing in.
    assert!(w.prepare().unwrap().nothing_to_send);
    assert_eq!(
        w.rows("SELECT count(*) FROM crate_entry", (), |r| r
            .get::<_, i64>(0)),
        [0]
    );
}

#[test]
fn rekordbox_entries_no_library_track_is_sent_as_are_counted_lost_and_need_the_confirm() {
    let (w, _, _) = peak_sent_and_imported();
    // The DJ added x.mp3, which isn't in the Library.
    w.rekordbox_shows(&[("Peak", &[1, 9, 2])]);

    let preflight = w.prepare().unwrap();

    assert_eq!(
        preflight.loses_entries,
        [LosesEntries {
            kind: TreeKind::Crate,
            path: vec!["Peak".to_owned()],
            lost: 1,
            entries: 3,
        }]
    );
    assert!(preflight.needs_confirm);
}

#[test]
fn undo_of_a_crate_change_rekordbox_overtook_is_refused_and_changes_nothing() {
    let (w, [a, b, c], peak) = peak_sent_and_imported();
    w.crate_op(move |conn| crates::add_on(conn, CrateId(peak), &[c]))
        .unwrap();
    w.send_through();
    // Imported, then the DJ takes c out again in rekordbox.
    w.rekordbox_shows(&[("Peak", &[1, 2, 3])]);
    w.send_through();
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, b.0]);

    match ops::undo_last_via(&w.writer).unwrap() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    assert_eq!(w.holds(peak), [a.0, b.0]);
}

#[test]
fn a_membership_settled_between_the_gos_merge_and_its_record_refuses_the_send() {
    // c's membership conflict: rekordbox's Peak lacks c, so the send leaves
    // c out of every crate it writes, and c isn't sent at all.
    let (w, _, _) = peak_with_no_base_and_both_sides_different();
    w.send_through();
    let file = fs::read(w.send_file()).unwrap();
    let on_c = w.open_ones()[1].id;
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    let preflight = w.prepare().unwrap();
    let writer = w.writer.clone();
    let sender = w.sender().before_write(move || {
        writer
            .call(move |c| Ok(conflicts::resolve(c, on_c, &Choice::KeepApp)))
            .unwrap()
            .unwrap();
    });

    assert_eq!(
        w.go_with(sender, &preflight.token, preflight.needs_confirm),
        Some(SendFailure::LibraryChanged)
    );
    assert_eq!(
        fs::read(w.send_file()).unwrap(),
        file,
        "the last file is back"
    );
    // The next send carries the membership chosen.
    w.send_through();
    assert_eq!(w.written("Peak"), ["a.mp3", "c.mp3", "b.mp3"]);
}

impl World {
    /// `sent_playlist.replaced` of every playlist recorded.
    fn replaced_records(&self) -> Vec<Option<String>> {
        self.rows(
            "SELECT replaced FROM sent_playlist WHERE kind = 'playlist' ORDER BY id",
            (),
            |r| r.get(0),
        )
    }
}

#[test]
fn a_playlist_a_send_emptied_is_seen_taken_up_and_the_djs_re_add_is_taken_in() {
    let w = World::new();
    let ids = ["a.mp3", "b.mp3", "c.mp3"].map(|f| w.library_track(f, f, true).0);
    let a = ids[0];
    let peak = w.crate_of("Peak", &[a]);
    w.rekordbox_shows(&[]);
    w.send_through();
    w.rekordbox_shows(&[("Peak", &[1])]);
    // The app empties Peak, and the send writes it empty.
    w.crate_op(move |conn| crates::remove_on(conn, CrateId(peak), &[a]))
        .unwrap();
    w.send_through();
    assert_eq!(w.replaced_records(), [Some(format!("[{}]", a.0))]);

    // Imported: rekordbox's Peak is empty, and the read sees it so.
    w.rekordbox_shows(&[("Peak", &[])]);
    w.prepare().unwrap();
    assert_eq!(w.replaced_records(), [None]);

    // The DJ puts a back in rekordbox: that's the DJ's change.
    w.rekordbox_shows(&[("Peak", &[1])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0]);
    assert_eq!(w.written("Peak"), ["a.mp3"]);
}

#[test]
fn a_send_from_an_older_export_answered_not_imported_records_what_it_replaced() {
    let (w, [a, b, c], peak) = peak_sent_and_imported();
    // A second send from that fresh export, nothing changed.
    w.send_through();
    w.crate_op(move |conn| crates::add_on(conn, CrateId(peak), &[c]))
        .unwrap();
    // The same export, now older than the last send: the user says nothing
    // was imported since, and the send goes.
    w.export_saved_at(Duration::from_secs(1_000_000_000));
    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    assert_eq!(w.go_answering(preflight.needs_confirm, Some(false)), None);
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3", "c.mp3"]);
    assert_eq!(w.replaced_records(), [Some(format!("[{}]", c.0))]);

    // Not imported; a fresh export still shows Peak without c.
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, b.0, c.0]);
    assert_eq!(w.written("Peak"), ["a.mp3", "b.mp3", "c.mp3"]);
}

#[test]
fn undo_of_a_removal_rekordbox_overtook_is_refused_and_brings_nothing_back_twice() {
    let (w, [a, b, _], peak) = peak_sent_and_imported();
    w.crate_op(move |conn| crates::remove_on(conn, CrateId(peak), &[b]))
        .unwrap();
    w.send_through();
    // Imported and read, then the DJ puts b back in rekordbox.
    w.rekordbox_shows(&[("Peak", &[1])]);
    w.prepare().unwrap();
    w.rekordbox_shows(&[("Peak", &[1, 2])]);
    w.send_through();
    assert_eq!(w.holds(peak), [a.0, b.0]);

    match ops::undo_last_via(&w.writer).unwrap() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    assert_eq!(w.holds(peak), [a.0, b.0]);
}
