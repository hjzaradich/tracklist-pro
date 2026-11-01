//! The deciding run's index of names (ROADMAP 1.4, "The name index"):
//! whatever way a file's names reach the index, every decision comes out
//! the same.

use super::support::*;
use crate::grouping::{Decided, Names};

/// About two minutes of made-up audio.
const LEN: usize = 900;

fn song<'a>(
    name: &'a str,
    title: Option<&'a str>,
    artist: Option<&'a str>,
    n: u8,
    items: Option<Vec<u32>>,
) -> NewFile<'a> {
    NewFile {
        name,
        title,
        artist,
        hash: Some(hash(n)),
        items,
        blake3: None,
    }
}

/// What the app kept after a step: tracks, pairs and their reasons, version
/// links, who is decided, and what the run counted. Never times or `rev`,
/// which differ between two databases.
fn kept(db: &Db, decided: Decided) -> Vec<String> {
    let mut out = vec![format!("{decided:?}")];
    out.push(format!("{:?}", db.groups()));
    out.push(format!("{:?}", db.decided()));
    db.writer
        .call(|c| {
            let mut pairs = c.prepare(
                "SELECT file_a || '-' || file_b || ' ' || outcome || ' ' || state || ' ' || reasons
                        || ' ' || coalesce(link_label, '-') || ' ' || coalesce(link_first, '-')
                 FROM grouping_pair ORDER BY file_a, file_b",
            )?;
            let pairs = pairs.query_map([], |r| r.get::<_, String>(0))?;
            let pairs: rusqlite::Result<Vec<String>> = pairs.collect();
            Ok(pairs?)
        })
        .map(|pairs| out.extend(pairs))
        .unwrap();
    out
}

/// Every kind of name, then everything that can happen to a file, deciding
/// after each change with `between` called just before. Returns what was
/// kept after every step.
fn every_step(db: &Db, between: fn(&Db)) -> Vec<Vec<String>> {
    let (velo, drive) = (track(1, LEN), track(2, LEN));
    // The same title with accents, in capitals, and with a featured artist.
    let accents = db.add(song(
        "Déjà Vu.flac",
        Some("Déjà Vu"),
        Some("Halden Rook"),
        1,
        Some(velo.clone()),
    ));
    let capitals = db.add(song(
        "DEJA VU.mp3",
        Some("DEJA VU"),
        Some("HALDEN ROOK"),
        2,
        Some(unproposed(&velo, 1)),
    ));
    db.add(song(
        "Deja Vu feat Quill Ashby.mp3",
        Some("Deja Vu (feat. Quill Ashby)"),
        Some("Halden Rook feat. Quill Ashby"),
        3,
        Some(reencoded(&velo, 2, 7)),
    ));
    // No title tag: the file name is all there is.
    db.add(song(
        "Halden Rook - Deja Vu (Radio Edit).mp3",
        None,
        None,
        4,
        Some(reencoded(&velo[300..700], 3, 7)),
    ));
    // Empty tags, no `raw_tags` at all, and `raw_tags` that can't be read.
    let empty = db.add(song("Deja Vu.ogg", Some(""), Some("   "), 5, None));
    let unread = db.add(song("Paper Harbor.wav", None, None, 6, None));
    db.sql(&format!(
        "UPDATE file SET raw_tags = NULL WHERE id = {empty};
         UPDATE file SET raw_tags = '{{\"id3v2\": [' WHERE id = {unread};"
    ));
    let none_at_all = db.add(song("Nightdrive.aiff", None, None, 7, None));
    db.sql(&format!(
        "UPDATE file SET raw_tags = NULL WHERE id = {none_at_all}"
    ));
    // A mashup naming two titles as its parts.
    db.add(song(
        "Deja Vu x Paper Harbor.mp3",
        Some("Deja Vu x Paper Harbor"),
        Some("Halden Rook"),
        8,
        Some(track(3, LEN)),
    ));
    let nightdrive = db.add(song(
        "Nightdrive.mp3",
        Some("Nightdrive"),
        Some("Quill Ashby"),
        9,
        Some(drive),
    ));
    // A title too many files hold: twelve of one artist are compared, 52 of
    // another and 52 with no artist are too many even so.
    for n in 0..116u8 {
        let name = format!("Intro {n}.mp3");
        let artist = match n {
            0..12 => Some("Paper Harbor"),
            12..64 => Some("Halden Rook"),
            _ => None,
        };
        db.add(song(&name, Some("Intro"), artist, 100 + n, None));
    }

    let mut steps = Vec::new();
    let mut decide = |step: &str| {
        between(db);
        let (decided, _) = db.decide();
        let mut seen = vec![step.to_string()];
        seen.extend(kept(db, decided));
        steps.push(seen);
        decided
    };
    let first = decide("everything is new");
    assert!(first.pairs > 0 && first.recorded > 0, "{first:?}");
    assert!(first.compared_directly > 0, "only the names found it");
    assert!(first.skipped_buckets > 0, "a title too many files share");
    assert_eq!(db.track(accents), db.track(capitals), "one title, one track");

    db.retag(nightdrive, Some("Déjà Vu"), Some("Halden Rook"));
    decide("retagged");
    db.sql(&format!(
        "UPDATE file SET rel_path = 'Paper Harbor.aiff', rel_path_key = 'Paper Harbor.aiff'
         WHERE id = {none_at_all}"
    ));
    decide("renamed");
    db.set_present(accents, false);
    decide("a file went missing");
    db.add(song(
        "Deja Vu (Club Mix).mp3",
        Some("Deja Vu (Club Mix)"),
        Some("Halden Rook"),
        10,
        Some(reencoded(&velo, 4, 7)),
    ));
    db.set_present(accents, true);
    decide("it came back, and a file arrived while it was gone");
    db.retag(capitals, Some(""), Some("   "));
    decide("its tags were emptied");
    db.add(song(
        "Nightdrive (Dub).mp3",
        Some("Nightdrive (Dub)"),
        Some("Quill Ashby"),
        11,
        None,
    ));
    decide("one new file");
    steps
}

fn nothing(_: &Db) {}

/// The app was closed and opened again: the index in memory is gone.
fn restart(db: &Db) {
    *db.names.lock().unwrap() = Names::default();
}

#[test]
fn a_restart_before_every_run_decides_exactly_what_an_unbroken_session_decides() {
    assert_eq!(every_step(&db(), restart), every_step(&db(), nothing));
}
