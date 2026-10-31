//! The send's review says and writes exactly what it did before 1cD-S1 made
//! it cheaper, for any library: the job the app runs and the review as it
//! was ([`review_before`], kept word for word for these tests) are held to
//! each other on the same database, preflight, token and file. The
//! libraries are made of every kind of track a send treats differently, in
//! orders a seed decides, over two sends and an old export. Every name is
//! invented.

use super::*;
use crate::conflicts::app_values;
use crate::send::preflight::review_before::{self, Before};

/// The kinds of track a send treats differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// rekordbox has it, at its file.
    Plain,
    /// rekordbox has it, and the app holds a value of its own.
    Edited,
    /// New, with no tag and no title: its file's name stands in.
    NoTitle,
    /// New, named by its tags.
    Tagged,
    /// New, and its file is gone: left out.
    GoneNew,
    /// rekordbox has it at the gone file's own `Location`: sent, and listed.
    GoneKnown,
    /// New, with a tag XML can't carry: left out.
    BadCharacter,
    /// New, in a format rekordbox isn't known to play.
    OddFormat,
    /// rekordbox holds another file of the track, not the linked one.
    Sibling,
    /// Sent as a byte-identical copy's entry.
    Copy,
    /// The file changed after the export, as the last scan recorded it.
    Late,
    /// rekordbox has it at another path, where no file is.
    Moved,
}

const KINDS: [Kind; 12] = [
    Kind::Plain,
    Kind::Edited,
    Kind::NoTitle,
    Kind::Tagged,
    Kind::GoneNew,
    Kind::GoneKnown,
    Kind::BadCharacter,
    Kind::OddFormat,
    Kind::Sibling,
    Kind::Copy,
    Kind::Late,
    Kind::Moved,
];

/// One `TRACK` of rekordbox's export.
struct Entry {
    id: u32,
    location: String,
    file: String,
    genre: String,
    rating: u32,
}

impl Entry {
    fn xml(&self) -> String {
        let Entry {
            id,
            location,
            file,
            genre,
            rating,
        } = self;
        format!(
            r#"<TRACK TrackID="{id}" Name="Theirs {file}" Artist="Kit" Genre="{genre}" TotalTime="200" Rating="{rating}" PlayCount="7" AverageBpm="128.00" Tonality="Am" Location="{location}"/>"#
        )
    }
}

/// A library, and what rekordbox has of it.
struct Varied {
    tracks: Vec<(LibraryTrackId, Kind)>,
    entries: Vec<Entry>,
    /// Per playlist of the export, the `TrackID`s in it.
    friday: Vec<u32>,
    warm_up: Vec<u32>,
}

impl Varied {
    fn export(&self, entries: usize) -> String {
        let tracks: Vec<String> = self.entries.iter().map(Entry::xml).collect();
        let keys = |ids: &[u32]| -> String {
            ids.iter()
                .map(|id| format!(r#"<TRACK Key="{id}"/>"#))
                .collect()
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="{entries}">{}</COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Type="0" Name="Crates" Count="2">
        <NODE Type="0" Name="Sets" Count="1">
          <NODE Name="Friday" Type="1" KeyType="0" Entries="{}">{}</NODE>
        </NODE>
        <NODE Name="Warm up" Type="1" KeyType="0" Entries="{}">{}</NODE>
      </NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#,
            tracks.join(""),
            self.friday.len(),
            keys(&self.friday),
            self.warm_up.len(),
            keys(&self.warm_up),
        )
    }

    /// The tracks of `kind`.
    fn of(&self, kind: Kind) -> Vec<LibraryTrackId> {
        self.tracks
            .iter()
            .filter(|(_, k)| *k == kind)
            .map(|(id, _)| *id)
            .collect()
    }
}

/// A small pseudo-random sequence: the same seed makes the same library.
struct Dice(u64);

impl Dice {
    fn next(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % below as u64).unwrap()
    }
}

impl World {
    /// A track of `kind`, the `n`th of the library. Returns the Library
    /// track and rekordbox's entry for it, if it has one.
    fn track_of(&self, kind: Kind, n: usize) -> (LibraryTrackId, Option<Entry>) {
        let id = u32::try_from(n).unwrap() + 100;
        let name = format!("{kind:?}-{n}.mp3").to_lowercase();
        let location = |file: &str| format!("file://localhost/E:/Music/{file}");
        let entry = |file: &str| {
            Some(Entry {
                id,
                location: location(file),
                file: file.to_owned(),
                genre: "House".to_owned(),
                rating: 102,
            })
        };
        match kind {
            Kind::Plain | Kind::Late => {
                let (track, file) = self.library_track(&name, "Ours", true);
                if kind == Kind::Late {
                    self.insert(
                        "UPDATE file SET mtime = 9000000000000000000 WHERE id = ?1",
                        (file,),
                    );
                }
                (track, entry(&name))
            }
            Kind::Edited => {
                let (track, _) = self.library_track(&name, "Ours", true);
                self.edit_genre(track, "Disco");
                (track, entry(&name))
            }
            Kind::NoTitle => {
                let recording = self.insert("INSERT INTO recording (title) VALUES (NULL)", ());
                let file = self.insert(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size,
                                       sniffed_format)
                     VALUES (1, ?1, ?1, 1, 1234, 'mp3')",
                    (name,),
                );
                self.link(recording, file);
                (LibraryTrackId(self.library_row(recording, file)), None)
            }
            Kind::Tagged => {
                let recording = self.insert("INSERT INTO recording (title) VALUES (NULL)", ());
                let file = self.file(recording, &name, "From its tag", true);
                self.insert(
                    "UPDATE file SET raw_tags = ?1 WHERE id = ?2",
                    (
                        json!({ "id3v2": [
                            {"key": "TIT2", "value": {"type": "text", "text": "From its tag"}},
                            {"key": "TPE1", "value": {"type": "text", "text": "Its artist"}},
                            {"key": "TCON", "value": {"type": "text", "text": "Garage"}},
                        ] })
                        .to_string(),
                        file,
                    ),
                );
                (LibraryTrackId(self.library_row(recording, file)), None)
            }
            Kind::GoneNew => (self.library_track(&name, "Gone", false).0, None),
            Kind::GoneKnown => (self.library_track(&name, "Gone", false).0, entry(&name)),
            Kind::BadCharacter => (self.library_track(&name, "Bad \u{1} title", true).0, None),
            Kind::OddFormat => {
                let (track, file) = self.library_track(&name, "Odd format", true);
                self.insert(
                    "UPDATE file SET sniffed_format = 'ogg' WHERE id = ?1",
                    (file,),
                );
                (track, None)
            }
            Kind::Sibling | Kind::Copy => {
                let other_name = format!("other-{name}");
                let recording = self.insert("INSERT INTO recording (title) VALUES (NULL)", ());
                let other = self.file(recording, &other_name, "The other", true);
                let linked = self.file(recording, &name, "The linked", true);
                if kind == Kind::Copy {
                    for file in [other, linked] {
                        self.insert(
                            "UPDATE file SET blake3 = ?2 WHERE id = ?1",
                            (file, vec![u8::try_from(n % 251).unwrap(); 32]),
                        );
                        self.insert(
                            "INSERT OR REPLACE INTO file_stage
                                 (file_id, stage, version, size, mtime, status)
                             SELECT id, 'hash', ?2, size, mtime, 'done' FROM file WHERE id = ?1",
                            (file, i64::from(crate::hash::DEFINITION)),
                        );
                    }
                }
                (
                    LibraryTrackId(self.library_row(recording, linked)),
                    entry(&other_name),
                )
            }
            Kind::Moved => {
                let (track, file) = self.library_track(&name, "Moved", true);
                self.insert(
                    "UPDATE file SET duration_ms = 200000 WHERE id = ?1",
                    (file,),
                );
                let mut entry = entry(&name).unwrap();
                entry.location = format!("file://localhost/C:/Old%20place/{name}");
                (track, Some(entry))
            }
        }
    }

    fn link(&self, recording: i64, file: i64) {
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            (recording, file),
        );
    }

    fn library_row(&self, recording: i64, file: i64) -> i64 {
        self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (recording, file),
        )
    }

    fn edit_genre(&self, track: LibraryTrackId, value: &'static str) {
        self.writer
            .call(move |c| Ok(app_values::set(c, &Plugged, track, "Genre", value)))
            .unwrap()
            .unwrap();
    }

    /// The tree a send writes: a folder holding a crate, and a crate at the
    /// top.
    fn crate_tree_of(&self, friday: &[LibraryTrackId], warm_up: &[LibraryTrackId]) {
        let sets = self.insert(
            "INSERT INTO crate (kind, name) VALUES ('folder', 'Sets')",
            (),
        );
        let inner = self.crate_of("Friday", friday);
        self.insert(
            "UPDATE crate SET parent_id = ?1 WHERE id = ?2",
            (sets, inner),
        );
        self.crate_of("Warm up", warm_up);
    }

    /// A library of these kinds, one track each, in order: nearly every
    /// track is in a crate, and rekordbox has one track no Library track is
    /// sent as, in a playlist.
    fn varied(&self, kinds: &[Kind]) -> Varied {
        let mut varied = Varied {
            tracks: Vec::new(),
            entries: Vec::new(),
            friday: vec![9],
            warm_up: Vec::new(),
        };
        let (mut friday, mut warm_up) = (Vec::new(), Vec::new());
        for (n, &kind) in kinds.iter().enumerate() {
            let (track, entry) = self.track_of(kind, n);
            varied.tracks.push((track, kind));
            let key = entry.as_ref().map(|e| e.id);
            varied.entries.extend(entry);
            if n % 7 == 6 {
                continue;
            }
            if n % 2 == 0 {
                friday.push(track);
                varied.friday.extend(key);
            } else {
                warm_up.push(track);
                varied.warm_up.extend(key);
            }
        }
        varied.entries.push(Entry {
            id: 9,
            location: "file://localhost/E:/Music/outsider.mp3".to_owned(),
            file: "outsider.mp3".to_owned(),
            genre: "House".to_owned(),
            rating: 0,
        });
        self.crate_tree_of(&friday, &warm_up);
        self.save_export_text(&varied.export(varied.entries.len()));
        varied
    }

    /// Prepares a send with the job the app runs, and holds what it gives
    /// to the review as it was, on the same database. Returns the old one.
    fn prepare_same_as_before(&self) -> Before {
        let preflight = self.prepare().expect("a preflight");
        let before = self
            .writer
            .call(|c| review_before::review(c, &Plugged))
            .unwrap()
            .expect("a read");
        // The prepare step's two looks at the disk, after the review.
        let mut expected = before.preflight.clone();
        expected.no_file_at_location = review_before::no_file_at_location(&before.sent_elsewhere);
        expected.changed_after_export =
            review_before::changed_on_disk(&before.watched, expected.export.modified_ms);
        assert_eq!(preflight, expected);
        assert_eq!(preflight.token, before.preflight.token);

        // The review the go runs says the same, whichever way it is run.
        let now = self
            .writer
            .call(|c| review(c, &Plugged))
            .unwrap()
            .expect("a read");
        assert_eq!(now.preflight, before.preflight);
        assert_eq!(now.outgoing, before.outgoing);
        assert_eq!(now.open, before.open);
        // What the prepare step looks up on disk, too.
        let elsewhere: Vec<(LibraryTrackId, String)> = before
            .sent_elsewhere
            .iter()
            .map(|sent| (sent.track.library_track, sent.path.clone()))
            .collect();
        let now_elsewhere: Vec<(LibraryTrackId, String)> = now
            .sent_elsewhere
            .iter()
            .map(|sent| (sent.track, sent.path.clone()))
            .collect();
        assert_eq!(now_elsewhere, elsewhere);
        let watched: Vec<(LibraryTrackId, Option<String>, bool)> = before
            .watched
            .iter()
            .map(|w| (w.track.library_track, w.path.clone(), w.recorded))
            .collect();
        let now_watched: Vec<(LibraryTrackId, Option<String>, bool)> = now
            .watched
            .iter()
            .map(|w| (w.track, w.path.clone(), w.recorded))
            .collect();
        assert_eq!(now_watched, watched);
        before
    }

    /// The go of the send `before` reviewed: it writes `before`'s bytes.
    fn go_same_as_before(&self, before: &Before, imported: Option<bool>) {
        assert_eq!(self.go_answering(true, imported), None);
        assert_eq!(
            fs::read(self.send_file()).unwrap(),
            before.outgoing.as_ref().unwrap().xml()
        );
    }

    /// rekordbox moves on after a send, the DJ too: stars, a genre changed
    /// on both sides, a genre changed only in rekordbox, tracks moved
    /// between playlists, energy sent as stars.
    fn the_next_week(&self, varied: &mut Varied) {
        for (n, entry) in varied.entries.iter_mut().enumerate() {
            if n % 2 == 0 {
                entry.genre = "Techno".to_owned();
            }
            if n % 3 == 0 {
                entry.rating = 204;
            }
        }
        for &track in &varied.of(Kind::Edited) {
            self.edit_genre(track, "Soul");
        }
        if let Some(moved) = varied.friday.pop() {
            varied.warm_up.push(moved);
        }
        for (n, &(track, _)) in varied.tracks.iter().enumerate() {
            if n % 2 == 1 {
                self.insert(
                    "INSERT INTO analysis (recording_id, source, energy)
                     SELECT recording_id, 'mik', ?2 FROM library_track WHERE id = ?1",
                    (track.0, i64::try_from(n % 10).unwrap() + 1),
                );
            }
            if n % 7 == 3 {
                self.insert(
                    "UPDATE library_track SET stars_held = 1 WHERE id = ?1",
                    (track.0,),
                );
            }
        }
        self.writer
            .call(|c| crate::energy_stars::set_on(c, true))
            .unwrap();
        self.save_export_text(&varied.export(varied.entries.len()));
    }
}

/// Two sends and an old export of one library: everything is the same as
/// before at every step.
fn check(kinds: &[Kind]) -> Vec<Preflight> {
    let w = World::new();
    let mut varied = w.varied(kinds);

    let first = w.prepare_same_as_before();
    w.go_same_as_before(&first, None);

    w.the_next_week(&mut varied);
    let second = w.prepare_same_as_before();
    w.go_same_as_before(&second, None);

    // An export saved before the last send: the user is asked.
    w.export_saved_at(Duration::from_secs(1_600_000_000));
    let old_export = w.prepare_same_as_before();
    w.go_same_as_before(&old_export, Some(false));
    vec![first.preflight, second.preflight, old_export.preflight]
}

#[test]
fn a_library_of_every_kind_of_track_gets_the_same_preflight_token_and_file_as_before() {
    let previews = check(&KINDS);
    let first = &previews[0];
    let second = &previews[1];
    // The library really goes down every road of the review.
    assert_eq!(first.left_out.len(), 2, "{:?}", first.left_out);
    assert_eq!(first.file_missing.len(), 1);
    assert_eq!(first.other_file.len(), 1);
    assert_eq!(first.no_kind.len(), 1);
    assert_eq!(first.changed_after_export.len(), 1);
    assert!(!first.loses_entries.is_empty());
    assert!(first.needs_confirm);
    assert_eq!(first.stars_changed, None);
    assert!(second.conflicts_waiting > 0);
    assert!(second.stars_changed.is_some());
    assert!(previews[2].export_older_than_last_send);
}

#[test]
fn libraries_in_a_random_order_get_the_same_preflight_token_and_file_as_before() {
    for seed in [
        0x9E37_79B9_7F4A_7C15,
        0xC2B2_AE3D_27D4_EB4F,
        0x1656_67B1_9E37_79F9,
    ] {
        let mut dice = Dice(seed);
        let kinds: Vec<Kind> = (0..40).map(|_| KINDS[dice.next(KINDS.len())]).collect();
        check(&kinds);
    }
}

#[test]
fn a_send_refused_as_a_whole_gets_the_same_preflight_and_token_as_before() {
    // Two crates of one name.
    let w = World::new();
    let varied = w.varied(&KINDS);
    w.crate_of("friday", &[]);
    w.crate_of("Friday ", &[]);
    let before = w.prepare_same_as_before();
    assert!(before.preflight.refusal.is_some());
    assert_eq!(before.outgoing, None);

    // An incomplete export: it says it holds more than it does.
    let w = World::new();
    let varied_again = w.varied(&KINDS);
    w.save_export_text(&varied_again.export(varied_again.entries.len() + 3));
    let before = w.prepare_same_as_before();
    assert_eq!(
        before.preflight.refusal.map(|r| r.reason),
        Some(RefusalReason::IncompleteExport)
    );
    drop(varied);
}

#[test]
fn nothing_to_send_gets_the_same_preflight_as_before() {
    let w = World::new();
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    let before = w.prepare_same_as_before();
    assert!(before.preflight.nothing_to_send);
}

/// Two gone files at one path, more than the merge's thousand tracks apart,
/// and a row of rekordbox's there. `in_a_crate` says which of the two tracks
/// a crate names. Returns the world and the two tracks.
fn two_gone_files_at_one_path(in_a_crate: [bool; 2]) -> (World, [LibraryTrackId; 2]) {
    let w = World::new();
    w.insert(
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
         VALUES (1, 'Music/Sets', 'Music/Sets')",
        (),
    );
    let first = w.insert("INSERT INTO recording (title) VALUES (NULL)", ());
    let file = w.file(first, "Sets/both.mp3", "One tag", false);
    let one = LibraryTrackId(w.library_row(first, file));
    w.writer
        .call(|c| {
            c.execute_batch(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1100)
                 INSERT INTO recording (title) SELECT NULL FROM n;
                 INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size,
                                   sniffed_format)
                 SELECT 1, 'filler-' || id || '.mp3', 'filler-' || id || '.mp3', 1, 1, 'mp3'
                 FROM recording WHERE id > 1;
                 INSERT INTO recording_file (recording_id, file_id, role)
                 SELECT r.id, f.id, 'best' FROM recording r
                 JOIN file f ON f.rel_path = 'filler-' || r.id || '.mp3';
                 INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                 SELECT recording_id, 'linked', file_id, 'ok' FROM recording_file
                 WHERE recording_id > 1",
            )
        })
        .unwrap();
    let last = w.insert("INSERT INTO recording (title) VALUES (NULL)", ());
    let other = w.insert(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size, sniffed_format,
                           raw_tags)
         SELECT 2, 'both.mp3', 'both.mp3', 0, 1, 'mp3', raw_tags FROM file WHERE id = ?1",
        (file,),
    );
    w.link(last, other);
    let two = LibraryTrackId(w.library_row(last, other));
    let tracks = [one, two];
    let named: Vec<LibraryTrackId> = (0..2)
        .filter(|&i| in_a_crate[i])
        .map(|i| tracks[i])
        .collect();
    w.crate_of("Both", &named);
    w.save_export(&[Rb(40, "Sets/both.mp3", "One row")]);
    (w, tracks)
}

// The merge reads a thousand tracks at a time and the first Library load read
// them all at once, so only the second saw that two files lead to one row of
// rekordbox's. The tracks of such files are listed by name, and the name must
// come out as the load of them all gave it, whichever tracks are listed.

#[test]
fn two_gone_files_at_one_path_far_apart_in_the_library_are_named_as_before() {
    let (w, [one, two]) = two_gone_files_at_one_path([true, true]);
    let before = w.prepare_same_as_before();
    let listed: Vec<LibraryTrackId> = before
        .preflight
        .left_out
        .iter()
        .map(|left| left.track.library_track)
        .chain(
            before
                .preflight
                .file_missing
                .iter()
                .map(|t| t.library_track),
        )
        .collect();
    assert!(listed.contains(&one) && listed.contains(&two), "{listed:?}");
}

#[test]
fn a_gone_file_sharing_its_path_with_a_track_no_crate_names_is_named_as_before() {
    let (w, [one, _]) = two_gone_files_at_one_path([true, false]);
    let before = w.prepare_same_as_before();
    assert_eq!(
        before
            .preflight
            .file_missing
            .iter()
            .map(|t| t.library_track)
            .collect::<Vec<_>>(),
        [one]
    );
}
