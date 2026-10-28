//! The merge before every send (1cB-3; ROADMAP 1.9 rule 2, 1.10), end to
//! end: rekordbox's export read, the app's own values merged, the file
//! written and the send recorded, then rekordbox read again. Every name and
//! value is invented.

use super::*;
use crate::conflicts::{self, app_values, Choice, Conflict, Filter};
use crate::ops::{self, UndoOutcome, UndoRefusal};

/// rekordbox's entry for `velo.mp3`, as each test starts: every attribute
/// a send must carry, analysis fields included.
const VELO: [(&str, &str); 7] = [
    ("Name", "Velo"),
    ("Artist", "Kit"),
    ("Genre", "House"),
    ("Rating", "204"),
    ("PlayCount", "7"),
    ("AverageBpm", "128.00"),
    ("Tonality", "Am"),
];

/// An export holding `velo.mp3` with [`VELO`]'s attributes changed by
/// `changes` (an empty value drops the attribute; a new name is added), and
/// `Entries` saying `entries`.
fn rekordbox_has_velo(changes: &[(&str, &str)], entries: usize) -> String {
    let mut attributes: Vec<(&str, &str)> = VELO.to_vec();
    for &(name, value) in changes {
        match attributes.iter_mut().find(|(n, _)| *n == name) {
            Some(there) => there.1 = value,
            None => attributes.push((name, value)),
        }
    }
    let attributes: String = attributes
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| format!(r#" {name}="{value}""#))
        .collect();
    export_holding(
        &format!(
            r#"<TRACK TrackID="40"{attributes} Location="file://localhost/E:/Music/velo.mp3"/>"#
        ),
        entries,
    )
}

/// An export whose COLLECTION holds this text and says it holds `entries`.
fn export_holding(tracks: &str, entries: usize) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="{entries}">
    {tracks}
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="0"/>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#
    )
}

impl World {
    /// rekordbox saves its export with `velo.mp3`'s attributes changed so.
    fn rekordbox_now(&self, changes: &[(&str, &str)]) {
        self.save_export_text(&rekordbox_has_velo(changes, 1));
    }

    /// The user edits a field in the app.
    fn edit(&self, track: LibraryTrackId, field: &str, value: &str) {
        let (field, value) = (field.to_owned(), value.to_owned());
        self.writer
            .call(move |c| Ok(app_values::set(c, &Plugged, track, &field, &value)))
            .unwrap()
            .unwrap();
    }

    /// A whole send: read the export, review, go. Returns the preflight.
    fn send(&self) -> Preflight {
        let preflight = self.prepare().unwrap();
        assert!(preflight.can_send, "{preflight:?}");
        assert_eq!(self.go(false), None);
        preflight
    }

    /// The attributes the last send file wrote for the track at
    /// `E:\Music\<file>`, in order.
    fn sent(&self, file: &str) -> Vec<(String, String)> {
        let read = RekordboxXml::parse(fs::read(self.send_file()).unwrap().as_slice()).unwrap();
        let track = read
            .tracks
            .iter()
            .find(|t| t.attrs.get("Location").is_some_and(|l| l.ends_with(file)))
            .unwrap_or_else(|| panic!("{file} wasn't sent"));
        track
            .attrs
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    fn sent_value(&self, file: &str, field: &str) -> Option<String> {
        self.sent(file)
            .into_iter()
            .find(|(name, _)| name == field)
            .map(|(_, value)| value)
    }

    fn query<T: Send + 'static>(
        &self,
        sql: &'static str,
        params: impl rusqlite::Params + Send + 'static,
        map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T> + Send + 'static,
    ) -> Vec<T> {
        self.writer
            .call(move |c| c.prepare(sql)?.query_map(params, map)?.collect())
            .unwrap()
    }

    /// The field's base: the value last sent.
    fn base(&self, track: LibraryTrackId, field: &'static str) -> Option<String> {
        self.query(
            "SELECT value FROM sync_base WHERE library_track_id = ?1 AND field = ?2",
            (track.0, field),
            |r| r.get(0),
        )
        .pop()
    }

    /// Every field that has a base.
    fn bases(&self, track: LibraryTrackId) -> Vec<String> {
        self.query(
            "SELECT field FROM sync_base WHERE library_track_id = ?1 ORDER BY field",
            (track.0,),
            |r| r.get(0),
        )
    }

    /// The app's own value of the field.
    fn own(&self, track: LibraryTrackId, field: &'static str) -> Option<String> {
        self.query(
            "SELECT value FROM app_value WHERE library_track_id = ?1 AND field = ?2",
            (track.0, field),
            |r| r.get(0),
        )
        .pop()
    }

    /// Every conflict's status, open or settled, oldest first.
    fn conflicts(&self) -> Vec<String> {
        self.query("SELECT status FROM conflict ORDER BY id", (), |r| r.get(0))
    }

    fn open_conflicts(&self) -> Vec<Conflict> {
        self.writer
            .call(|c| conflicts::open_conflicts(c, &Filter::default()))
            .unwrap()
    }

    /// Settles the one open conflict.
    fn resolve(&self, choice: Choice) {
        let open = self.open_conflicts();
        assert_eq!(open.len(), 1, "{open:?}");
        let id = open[0].id;
        self.writer
            .call(move |c| Ok(conflicts::resolve(c, id, &choice)))
            .unwrap()
            .unwrap();
    }
}

/// The three values of a conflict.
fn sides_of(conflict: &Conflict) -> (Option<&str>, Option<&str>, Option<&str>) {
    (
        conflict.app_value.as_deref(),
        conflict.rekordbox_value.as_deref(),
        conflict.base_value.as_deref(),
    )
}

/// `velo.mp3`, a track rekordbox has, in a crate, sent once as rekordbox
/// has it: every base is rekordbox's value.
fn velo_sent_once() -> (World, LibraryTrackId) {
    let w = World::new();
    let (velo, _) = w.library_track("velo.mp3", "Velo", true);
    w.crate_of("Peak", &[velo]);
    w.rekordbox_now(&[]);
    w.send();
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("House"));
    (w, velo)
}

/// [`velo_sent_once`], then the Genre edited on both sides: Deep House in
/// the app, Tech House in rekordbox. The send of that has gone.
fn velo_edited_on_both_sides() -> (World, LibraryTrackId) {
    let (w, velo) = velo_sent_once();
    w.edit(velo, "Genre", "Deep House");
    w.rekordbox_now(&[("Genre", "Tech House")]);
    let preflight = w.send();
    assert_eq!(preflight.conflicts_waiting, 1);
    (w, velo)
}

// ---- One side changed ---------------------------------------------------------

#[test]
fn only_the_apps_edit_goes_out_with_every_other_attribute_rekordboxs_and_the_base_moves() {
    let (w, velo) = velo_sent_once();
    w.edit(velo, "Genre", "Deep House");

    let preflight = w.send();

    assert_eq!(preflight.conflicts_waiting, 0);
    let expected: Vec<(String, String)> = [
        ("TrackID", "40"),
        ("Name", "Velo"),
        ("Artist", "Kit"),
        ("Genre", "Deep House"),
        ("Rating", "204"),
        ("PlayCount", "7"),
        ("Location", "file://localhost/E:/Music/velo.mp3"),
    ]
    .iter()
    .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
    .collect();
    assert_eq!(w.sent("velo.mp3"), expected, "analysis fields never sent");
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Deep House"));
    assert_eq!(
        w.bases(velo),
        ["Artist", "Genre", "Location", "Name", "PlayCount", "Rating"],
        "no analysis field has a base"
    );
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.conflicts(), Vec::<String>::new());
}

#[test]
fn only_rekordboxs_change_is_taken_in_the_apps_value_goes_and_rekordboxs_is_sent() {
    let (w, velo) = velo_sent_once();
    w.edit(velo, "Genre", "Deep House");
    w.send();
    // Imported, then changed in rekordbox.
    w.rekordbox_now(&[("Genre", "Tech House")]);

    let preflight = w.send();

    assert_eq!(preflight.conflicts_waiting, 0);
    assert_eq!(w.own(velo, "Genre"), None);
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.conflicts(), Vec::<String>::new());

    // And it stays rekordbox's: the next send carries it again.
    w.send();
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
}

#[test]
fn undo_of_an_edit_is_refused_after_rekordboxs_value_was_taken_in() {
    let (w, velo) = velo_sent_once();
    w.edit(velo, "Genre", "Deep House");
    w.send();
    w.rekordbox_now(&[("Genre", "Tech House")]);
    w.send();
    assert_eq!(w.own(velo, "Genre"), None);

    match ops::undo_last_via(&w.writer).unwrap() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    assert_eq!(w.own(velo, "Genre"), None);
}

#[test]
fn a_send_never_imported_sends_the_apps_value_again() {
    let (w, velo) = velo_sent_once();
    w.edit(velo, "Genre", "Deep House");
    w.send();
    // The file wasn't imported: rekordbox still shows House.
    w.rekordbox_now(&[]);

    w.send();

    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.conflicts(), Vec::<String>::new());
}

#[test]
fn a_new_tracks_own_values_go_out_over_its_files() {
    let w = World::new();
    let (track, _) = w.library_track("new.mp3", "New", true);
    w.crate_of("Peak", &[track]);
    w.save_export(&[]);
    w.edit(track, "Name", "Renamed");
    w.edit(track, "Genre", "Disco");

    let preflight = w.send();

    assert_eq!(preflight.new_tracks, 1);
    assert_eq!(w.sent_value("new.mp3", "Name").as_deref(), Some("Renamed"));
    assert_eq!(w.sent_value("new.mp3", "Genre").as_deref(), Some("Disco"));
    assert_eq!(w.base(track, "Genre").as_deref(), Some("Disco"));
}

// ---- Both changed: an open conflict -------------------------------------------

#[test]
fn both_changed_opens_a_conflict_sends_rekordboxs_value_and_keeps_the_base() {
    let (w, velo) = velo_edited_on_both_sides();

    let open = w.open_conflicts();
    assert_eq!(open.len(), 1);
    assert_eq!(
        sides_of(&open[0]),
        (Some("Deep House"), Some("Tech House"), Some("House"))
    );
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("House"));
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Deep House"));

    // rekordbox imported that send: it shows Tech House, as before. The
    // conflict is still the same open one, and the app's value still waits.
    w.rekordbox_now(&[("Genre", "Tech House")]);
    let preflight = w.send();

    assert_eq!(preflight.conflicts_waiting, 1);
    assert_eq!(w.open_conflicts(), open);
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("House"));
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Deep House"));
}

#[test]
fn a_field_never_sent_with_an_open_conflict_gets_no_base() {
    let (w, velo) = velo_sent_once();
    // Comments: rekordbox had none when the app set one, and has its own now.
    w.edit(velo, "Comments", "warm-up");
    w.rekordbox_now(&[("Comments", "closer")]);

    let preflight = w.send();

    assert_eq!(preflight.conflicts_waiting, 1);
    assert_eq!(
        sides_of(&w.open_conflicts()[0]),
        (Some("warm-up"), Some("closer"), None)
    );
    assert_eq!(w.sent_value("velo.mp3", "Comments").as_deref(), Some("closer"));
    assert_eq!(w.base(velo, "Comments"), None);

    w.send();
    assert_eq!(w.open_conflicts().len(), 1, "still waiting after the next read");
    assert_eq!(w.base(velo, "Comments"), None);
}

// ---- A settled conflict's value goes out and stays ----------------------------

#[test]
fn keeping_the_apps_value_sends_it_and_it_stays() {
    let (w, velo) = velo_edited_on_both_sides();
    w.resolve(Choice::KeepApp);

    w.send();

    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Deep House"));

    // Imported: rekordbox shows Deep House. Nothing is open, nothing new.
    w.rekordbox_now(&[("Genre", "Deep House")]);
    let preflight = w.send();
    assert_eq!(preflight.conflicts_waiting, 0);
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.open_conflicts(), []);
    assert_eq!(w.conflicts(), ["kept_app"], "only the settled one");
}

#[test]
fn keeping_rekordboxs_value_sends_it_and_the_value_turned_down_never_goes_out() {
    let (w, velo) = velo_edited_on_both_sides();
    w.resolve(Choice::KeepRekordbox);
    assert_eq!(w.own(velo, "Genre"), None);

    w.send();

    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Tech House"));

    // The next read and send: nothing is sent over again, no conflict.
    w.rekordbox_now(&[("Genre", "Tech House")]);
    let preflight = w.send();
    assert_eq!(preflight.conflicts_waiting, 0);
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.open_conflicts(), []);
    assert_eq!(w.conflicts(), ["kept_rekordbox"], "only the settled one");
}

#[test]
fn a_value_given_by_hand_is_sent_and_it_stays() {
    let (w, velo) = velo_edited_on_both_sides();
    w.resolve(Choice::Edit("Disco".into()));

    w.send();

    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Disco"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Disco"));
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Disco"));

    w.rekordbox_now(&[("Genre", "Disco")]);
    w.send();
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Disco"));
    assert_eq!(w.open_conflicts(), []);
}

#[test]
fn undo_of_a_resolve_is_refused_once_a_send_carried_the_value_chosen() {
    for choice in [
        Choice::KeepApp,
        Choice::KeepRekordbox,
        Choice::Edit("Disco".into()),
    ] {
        let (w, velo) = velo_edited_on_both_sides();
        w.resolve(choice.clone());
        w.send();
        let own = w.own(velo, "Genre");

        match ops::undo_last_via(&w.writer).unwrap() {
            UndoOutcome::Refused { reason, .. } => {
                assert_eq!(reason, UndoRefusal::SentSince, "{choice:?}");
            }
            other => panic!("{choice:?}: expected the undo to be refused, got {other:?}"),
        }
        assert_eq!(w.open_conflicts(), [], "{choice:?}");
        assert_eq!(w.own(velo, "Genre"), own, "{choice:?}");
    }
}

#[test]
fn undo_of_a_resolve_before_the_next_send_reopens_the_conflict_and_the_app_value_waits_again() {
    let (w, velo) = velo_edited_on_both_sides();
    let open = w.open_conflicts();
    w.resolve(Choice::KeepRekordbox);

    match ops::undo_last_via(&w.writer).unwrap() {
        UndoOutcome::Undone { .. } => {}
        other => panic!("expected the undo, got {other:?}"),
    }
    assert_eq!(w.open_conflicts(), open);
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Deep House"));

    let preflight = w.send();
    assert_eq!(preflight.conflicts_waiting, 1);
    assert_eq!(w.sent_value("velo.mp3", "Genre").as_deref(), Some("Tech House"));
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("House"));
}

// ---- A read that can't be trusted decides nothing -----------------------------

#[test]
fn a_row_kept_from_an_older_read_takes_nothing_in_and_opens_nothing() {
    let (w, velo) = velo_sent_once();
    w.edit(velo, "Genre", "Deep House");
    w.send();
    // A read shows Tech House (landing from elsewhere, so no send merged
    // it), then an incomplete export leaves velo out: its row is kept.
    another_read(
        &w.writer,
        rekordbox_has_velo(&[("Genre", "Tech House")], 1),
        None,
        "2026-01-01T00:00:00.000Z",
    );
    let incomplete = export_holding(
        r#"<TRACK TrackID="41" Name="Other" Location="file://localhost/E:/Music/other.mp3"/>"#,
        2,
    );
    w.save_export_text(&incomplete);
    let bases = w.bases(velo);

    let preflight = w.prepare().unwrap();

    assert!(preflight.refusal.is_some(), "an incomplete export is refused");
    // velo is still rekordbox's, by the row kept from the earlier read.
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 1));
    assert_eq!(w.own(velo, "Genre").as_deref(), Some("Deep House"));
    assert_eq!(w.conflicts(), Vec::<String>::new());
    assert_eq!(w.bases(velo), bases);
    assert_eq!(w.base(velo, "Genre").as_deref(), Some("Deep House"));
}
