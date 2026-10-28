//! What a send records for the engine (1cB-19): what each base replaced in
//! rekordbox, used for a send never imported (decision table row 7) and
//! forgotten once a read shows the send taken up.

use super::*;
use crate::rekordbox::location::decode;
use crate::rekordbox_write::{forget_taken_up, record_send, SentTrack};

/// Reads after [`THIS_READ`], in order.
const NEXT_READ: &str = "2026-10-19T09:00:00.000Z";
const LAST_READ: &str = "2026-10-26T09:00:00.000Z";

/// The `Location` rekordbox holds a track at, and the send sends it to.
fn location_of(id: LibraryTrackId) -> String {
    format!("file://localhost/E:/Music/Track%20{}.mp3", id.0)
}

/// A track's base: field, value sent, and what that replaced.
type Record = (String, String, Option<String>);

fn record(field: &str, value: &str, replaced: Option<&str>) -> Record {
    (field.into(), value.into(), replaced.map(Into::into))
}

impl Lib {
    /// The track's entry in the read made at `read_at`, at its own
    /// `Location`, a trusted match, holding these attributes. Only the row:
    /// what a read does about sends isn't run.
    fn place(&self, id: LibraryTrackId, read_at: &'static str, attributes: &[(&str, &str)]) {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        let location = location_of(id);
        let key = decode(&location).unwrap().match_key();
        let mut json = serde_json::Map::new();
        json.insert("TrackID".into(), track_id.to_string().into());
        for (name, value) in attributes {
            json.insert((*name).into(), (*value).into());
        }
        json.insert("Location".into(), location.into());
        let json = serde_json::Value::Object(json).to_string();
        self.writer
            .call(move |c| {
                let file: i64 = c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )?;
                c.execute("DELETE FROM rekordbox_track WHERE file_id = ?1", [file])?;
                c.execute(
                    "INSERT INTO rekordbox_track
                         (attributes, location_key, read_at, file_id, relink_method,
                          relink_probable)
                     VALUES (?1, ?2, ?3, ?4, 'path', 0)",
                    (json, key, read_at, file),
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// A read made at `read_at` showing the track's entry with these
    /// attributes: the row, then what every read does about sends, in one
    /// step.
    fn read(&self, id: LibraryTrackId, read_at: &'static str, attributes: &[(&str, &str)]) {
        self.place(id, read_at, attributes);
        self.writer
            .call(move |c| forget_taken_up(c, read_at))
            .unwrap();
    }

    /// A Library track rekordbox knows, holding these attributes in
    /// [`THIS_READ`].
    fn in_rekordbox(&self, attributes: &[(&str, &str)]) -> LibraryTrackId {
        let (id, _) = self.linked();
        self.place(id, THIS_READ, attributes);
        id
    }

    /// A send of the track with these fields (as the merge before a send is
    /// to fill them), recorded. `in_rekordbox`: the send knew rekordbox's
    /// entry for it.
    fn send_as(&self, id: LibraryTrackId, in_rekordbox: bool, fields: &[(&str, &str)]) {
        let mut attributes = vec![("TrackID".to_owned(), "1".to_owned())];
        attributes.extend(
            fields
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
        );
        attributes.push(("Location".into(), location_of(id)));
        let sent = SentTrack {
            library_track: id,
            in_rekordbox,
            track_id: 1,
            attributes,
            rekordbox_holds_other_file: None,
            file_missing: false,
        };
        self.writer
            .call(move |c| record_send(c, &[sent], &[]))
            .unwrap();
    }

    fn send(&self, id: LibraryTrackId, fields: &[(&str, &str)]) {
        self.send_as(id, true, fields);
    }

    fn records(&self, id: LibraryTrackId) -> Vec<Record> {
        self.writer
            .call(move |c| {
                c.prepare(
                    "SELECT field, value, replaced FROM sync_base WHERE library_track_id = ?1
                     ORDER BY field",
                )?
                .query_map([id.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
            })
            .unwrap()
    }
}

/// The outcome detection gives one field of a track.
fn outcome_of<'a>(detected: &'a Detected, field: &str) -> &'a FieldOutcome {
    detected
        .fields
        .iter()
        .find(|f| f.field == field)
        .unwrap_or_else(|| panic!("no {field} in {detected:?}"))
}

#[test]
fn a_send_records_what_each_base_replaced_in_rekordbox() {
    let lib = Lib::new();
    let id = lib.in_rekordbox(&[("Name", "Velo"), ("Genre", "House")]);

    lib.send(
        id,
        &[
            ("Name", "Velo"),
            ("Genre", "Deep House"),
            ("Comments", "warm-up"),
            ("Rating", "0"),
        ],
    );

    let location = location_of(id);
    assert_eq!(
        lib.records(id),
        [
            // rekordbox's entry had no Comments.
            record("Comments", "warm-up", Some("")),
            record("Genre", "Deep House", Some("House")),
            // Sent as rekordbox has them, by the field's kind (no rating is
            // 0 stars): nothing replaced.
            record("Location", &location, None),
            record("Name", "Velo", None),
            record("Rating", "0", None),
        ]
    );

    // A track the send took for new: no trusted entry, nothing recorded,
    // even where a row of the read is at its Location.
    let new = lib.in_rekordbox(&[("Genre", "House")]);
    lib.send_as(new, false, &[("Genre", "Deep House")]);
    assert_eq!(
        lib.records(new),
        [
            record("Genre", "Deep House", None),
            record("Location", &location_of(new), None),
        ]
    );

    // An entry that isn't of the newest read isn't what rekordbox shows.
    let kept = lib.in_rekordbox(&[("Genre", "House")]);
    let (other, _) = lib.linked();
    lib.place(other, NEXT_READ, &[("Genre", "Techno")]);
    lib.send(kept, &[("Genre", "Deep House")]);
    assert_eq!(lib.records(kept)[0], record("Genre", "Deep House", None));

    // The next send records again: a base sent as rekordbox shows it now
    // replaced nothing.
    lib.place(id, NEXT_READ, &[("Name", "Velo"), ("Genre", "Deep House")]);
    lib.send(id, &[("Name", "Velo"), ("Genre", "Deep House")]);
    assert_eq!(lib.records(id)[0], record("Genre", "Deep House", None));
}

#[test]
fn a_send_never_imported_stands_by_what_the_send_replaced_not_what_the_edit_did() {
    let lib = Lib::new();
    let id = lib.in_rekordbox(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.send(id, &[("Genre", "Deep House")]);
    // Only the send's record may count: the edit's says something else.
    lib.run("UPDATE app_value SET replaced = 'Disco'");

    // The next read still shows House: the XML wasn't imported.
    lib.read(id, NEXT_READ, &[("Genre", "House")]);

    assert_eq!(
        lib.records(id)[0],
        record("Genre", "Deep House", Some("House"))
    );
    let detected = lib.detect_stored(&[id]);
    assert_eq!(lib.rows(), []);
    let genre = outcome_of(&detected, "Genre");
    assert_eq!(genre.decision.verdict, Verdict::NotTakenUp);
    assert_eq!(genre.outcome, Outcome::SendApp);

    // Without the send's record, House is rekordbox's change, whatever the
    // edit replaced.
    lib.run("UPDATE sync_base SET replaced = NULL; UPDATE app_value SET replaced = 'House';");
    let detected = lib.detect_stored(&[id]);
    let genre = outcome_of(&detected, "Genre");
    assert_eq!(genre.decision.reason, Reason::OnlyRekordboxChanged);
    assert_eq!(genre.outcome, Outcome::TakeRekordbox);
}

#[test]
fn an_imported_send_changed_back_in_rekordbox_is_rekordboxs_change_never_sent_again() {
    let lib = Lib::new();
    let id = lib.in_rekordbox(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.set(id, "Comments", "warm-up").unwrap();
    lib.send(id, &[("Genre", "Deep House"), ("Comments", "warm-up")]);
    assert_eq!(
        lib.records(id)[..2],
        [
            record("Comments", "warm-up", Some("")),
            record("Genre", "Deep House", Some("House")),
        ]
    );

    // Imported: the next read shows what was sent, and the records go.
    lib.read(
        id,
        NEXT_READ,
        &[("Genre", "Deep House"), ("Comments", "warm-up")],
    );
    assert!(
        lib.records(id)
            .iter()
            .all(|(_, _, replaced)| replaced.is_none()),
        "{:?}",
        lib.records(id)
    );

    // The DJ sets Genre back in rekordbox and Comments to something else,
    // while the app changes Comments too.
    lib.set(id, "Comments", "peak time").unwrap();
    lib.read(
        id,
        LAST_READ,
        &[("Genre", "House"), ("Comments", "closing")],
    );
    let detected = lib.detect_stored(&[id]);

    let genre = outcome_of(&detected, "Genre");
    assert_eq!(genre.decision.reason, Reason::OnlyRekordboxChanged);
    assert_eq!(genre.outcome, Outcome::TakeRekordbox);
    let open = lib.only_open();
    assert_eq!(open.field, "Comments");
    assert_eq!(
        values(&open),
        (Some("peak time"), Some("closing"), Some("warm-up"))
    );
    assert!(
        detected
            .fields
            .iter()
            .all(|f| f.outcome != Outcome::SendApp),
        "{detected:?}"
    );
}

#[test]
fn a_read_showing_one_field_of_a_send_taken_up_forgets_all_it_replaced() {
    // rekordbox imports a whole entry: if one field the send changed shows
    // the value sent, the send was taken up, and a field that shows what
    // it replaced was set back since.
    let lib = Lib::new();
    let id = lib.in_rekordbox(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.set(id, "Comments", "warm-up").unwrap();
    lib.send(id, &[("Genre", "Deep House"), ("Comments", "warm-up")]);

    // Imported, and Genre set back before the next read.
    lib.read(
        id,
        NEXT_READ,
        &[("Genre", "House"), ("Comments", "warm-up")],
    );

    assert!(
        lib.records(id)
            .iter()
            .all(|(_, _, replaced)| replaced.is_none()),
        "{:?}",
        lib.records(id)
    );
    let detected = lib.detect_stored(&[id]);
    let genre = outcome_of(&detected, "Genre");
    assert_eq!(genre.decision.reason, Reason::OnlyRekordboxChanged);
    assert_eq!(genre.outcome, Outcome::TakeRekordbox);
    assert_eq!(lib.rows(), []);
}

#[test]
fn a_row_kept_from_an_older_read_forgets_nothing_and_decides_nothing() {
    let lib = Lib::new();
    let id = lib.in_rekordbox(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.send(id, &[("Genre", "Deep House")]);
    // The track's row shows the value sent, but a newer, incomplete read
    // (of another track) kept it from an older read (§5.3).
    lib.place(id, THIS_READ, &[("Genre", "Deep House")]);
    let (other, _) = lib.linked();
    lib.read(other, NEXT_READ, &[("Genre", "Techno")]);

    assert_eq!(
        lib.records(id)[0],
        record("Genre", "Deep House", Some("House"))
    );
    let detected = lib.detect_stored(&[id]);
    let genre = outcome_of(&detected, "Genre");
    assert_eq!(genre.decision.verdict, Verdict::Wait);
    assert_eq!(genre.outcome, Outcome::Wait);
    assert_eq!(lib.rows(), []);

    // The same row in a read of its own does forget.
    lib.read(id, NEXT_READ, &[("Genre", "Deep House")]);
    assert_eq!(lib.records(id)[0], record("Genre", "Deep House", None));
}

#[test]
fn two_rows_of_a_read_at_the_location_sent_forget_nothing() {
    let lib = Lib::new();
    let id = lib.in_rekordbox(&[("Genre", "House")]);
    lib.send(id, &[("Genre", "Deep House")]);
    let location = location_of(id);
    let key = decode(&location).unwrap().match_key();
    let attributes = serde_json::json!({
        "TrackID": "900",
        "Genre": "Deep House",
        "Location": location,
    })
    .to_string();
    lib.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO rekordbox_track (attributes, location_key, read_at)
                 VALUES (?1, ?2, ?3)",
                (attributes, key, NEXT_READ),
            )
        })
        .unwrap();

    lib.read(id, NEXT_READ, &[("Genre", "Deep House")]);

    assert_eq!(
        lib.records(id)[0],
        record("Genre", "Deep House", Some("House"))
    );
}
