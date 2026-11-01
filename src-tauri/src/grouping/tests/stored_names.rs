//! The deciding run's index of names (ROADMAP 1.4, "The name index"):
//! whatever way a file's names reach the index, every decision comes out
//! the same.

use std::sync::mpsc;
use std::time::Duration;

use rusqlite::OptionalExtension;

use super::support::*;
use crate::db::{DbError, ReadPool};
use crate::grouping::decide::Entry;
use crate::grouping::stored_names::{keep, WorkedOut};
use crate::grouping::{Decided, Names, RULES_VERSION};

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
            pairs
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
    // Empty tags, no `raw_tags` at all, and `raw_tags` of a shape nobody
    // wrote (the table takes any JSON object).
    let empty = db.add(song("Deja Vu.ogg", Some(""), Some("   "), 5, None));
    let unread = db.add(song("Paper Harbor.wav", None, None, 6, None));
    db.sql(&format!(
        "UPDATE file SET raw_tags = NULL WHERE id = {empty};
         UPDATE file SET raw_tags = '{{\"id3v2\": 7}}' WHERE id = {unread};"
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
    assert_eq!(
        db.track(accents),
        db.track(capitals),
        "one title, one track"
    );

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

/// The app was closed and opened again with nothing stored: every file's
/// names are parsed from `raw_tags`, all the app could do before 0045.
fn restart_the_old_way(db: &Db) {
    restart(db);
    db.sql("DELETE FROM grouping_names");
}

#[test]
fn a_restart_before_every_run_decides_exactly_what_an_unbroken_session_decides() {
    assert_eq!(every_step(&db(), restart), every_step(&db(), nothing));
}

#[test]
fn reading_stored_keys_decides_exactly_what_parsing_every_files_tags_decides() {
    assert_eq!(
        every_step(&db(), restart),
        every_step(&db(), restart_the_old_way)
    );
}

/// The index a run would build now, on a connection of its own.
fn index(db: &Db) -> Names {
    let reads = ReadPool::with_size(db.writer.guarded_path(), 1).unwrap();
    let mut names = Names::default();
    names
        .catch_up::<DbError>(&db.writer, &reads, &mut |_| Ok(()), &mut || Ok(()))
        .unwrap();
    names
}

/// What `grouping_names` holds for `file`.
fn stored(db: &Db, file: i64) -> Option<String> {
    db.writer
        .call(move |c| {
            c.query_row(
                "SELECT entry FROM grouping_names WHERE file_id = ?1",
                [file],
                |r| r.get(0),
            )
            .optional()
        })
        .unwrap()
}

fn lone<'a>(name: &'a str, title: &'a str, n: u8) -> NewFile<'a> {
    song(name, Some(title), Some("Halden Rook"), n, None)
}

/// A file with a record and its keys stored, as a run leaves it.
fn decided_file(db: &Db, name: &str, title: &str, n: u8) -> i64 {
    let file = db.add(lone(name, title, n));
    db.group();
    index(db);
    file
}

fn titles(names: &Names, file: i64) -> Vec<String> {
    names.entry_of(file).expect("in the index").titles.clone()
}

#[test]
fn after_a_restart_one_new_file_is_the_only_one_whose_tags_are_parsed() {
    let db = db();
    for n in 1..=5u8 {
        db.add(lone(&format!("{n}.mp3"), &format!("Song {n}"), n));
    }
    db.decide();
    assert_eq!(db.names.lock().unwrap().worked_out(), 5, "the first run");
    restart(&db);
    db.add(lone("6.mp3", "Song 6", 6));
    db.decide();
    let names = db.names.lock().unwrap();
    assert_eq!(names.files(), 6);
    assert_eq!(names.worked_out(), 1);
}

#[test]
fn the_stored_keys_are_exactly_the_keys_every_files_tags_give() {
    let db = db();
    every_step(&db, nothing);
    let from_stored = index(&db);
    assert_eq!(from_stored.worked_out(), 0, "every file's keys are stored");
    restart_the_old_way(&db);
    let from_tags = index(&db);
    assert_eq!(from_tags.files(), from_stored.files());
    assert_eq!(from_tags.worked_out() as usize, from_tags.files());
    for file in 1..=db.count("SELECT max(id) FROM file") {
        assert_eq!(
            from_stored.entry_of(file),
            from_tags.entry_of(file),
            "{file}"
        );
    }
}

#[test]
fn the_index_reads_a_current_row_and_not_the_tags() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.sql("UPDATE grouping_names SET entry = '[[\"written by the test\"],[],[]]'");
    let names = index(&db);
    assert_eq!(names.worked_out(), 0);
    assert_eq!(titles(&names, file), ["written by the test"]);
}

#[test]
fn a_retag_makes_the_stored_keys_worked_out_again() {
    let db = db();
    let file = decided_file(&db, "Track.mp3", "Velo", 1);
    assert!(stored(&db, file).unwrap().contains("velo"));
    db.retag(file, Some("Nightdrive"), Some("Halden Rook"));
    let names = index(&db);
    assert_eq!(names.worked_out(), 1);
    assert!(titles(&names, file).contains(&"nightdrive".to_string()));
    let now = stored(&db, file).unwrap();
    assert!(now.contains("nightdrive") && !now.contains("velo"), "{now}");
}

#[test]
fn a_rename_makes_the_stored_keys_worked_out_again() {
    let db = db();
    let file = db.add(song("Velo.mp3", None, None, 1, None));
    db.group();
    index(&db);
    db.sql("UPDATE file SET rel_path = 'Nightdrive.mp3', rel_path_key = 'Nightdrive.mp3'");
    let names = index(&db);
    assert_eq!(titles(&names, file), ["nightdrive"]);
    assert_eq!(stored(&db, file).unwrap(), r#"[["nightdrive"],[],[]]"#);
}

#[test]
fn a_rescan_that_finds_other_audio_makes_the_stored_keys_worked_out_again() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.overwrite(file, Some(hash(2)), None);
    assert_eq!(index(&db).worked_out(), 1);
}

#[test]
fn a_rescan_that_changes_nothing_leaves_the_stored_keys_alone() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.retag(file, Some("Velo"), Some("Halden Rook"));
    assert_eq!(index(&db).worked_out(), 0);
}

#[test]
fn keys_stored_by_other_rules_are_worked_out_again_and_never_read() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.sql(&format!(
        "UPDATE grouping_names SET rules_version = {}, entry = '[[\"old rules\"],[],[]]'",
        RULES_VERSION - 1
    ));
    let names = index(&db);
    assert_eq!(names.worked_out(), 1);
    assert_eq!(titles(&names, file), ["velo"]);
    assert_eq!(
        db.count("SELECT rules_version FROM grouping_names"),
        i64::from(RULES_VERSION)
    );
}

#[test]
fn keys_stored_at_another_rev_are_worked_out_again_and_never_read() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.sql("UPDATE grouping_names SET rev = rev + 1, entry = '[[\"old names\"],[],[]]'");
    assert_eq!(titles(&index(&db), file), ["velo"]);
}

#[test]
fn a_run_with_keys_behind_catches_up_before_it_decides() {
    let db = db();
    let old = db.add(lone("Velo.flac", "Velo", 1));
    db.decide();
    db.sql("UPDATE grouping_names SET rules_version = 0, entry = '[[\"old rules\"],[],[]]'");
    restart(&db);
    let new = db.add(lone("Velo.mp3", "Velo", 2));
    let (decided, _) = db.decide();
    assert_eq!(decided.files, 1, "only the new file is due");
    assert_eq!(decided.pairs, 1, "and its title found the old file");
    assert_eq!(titles(&db.names.lock().unwrap(), old), ["velo"]);
    assert!(stored(&db, new).is_some());
}

#[test]
fn a_file_retagged_while_its_keys_were_worked_out_keeps_none_of_its_old_names() {
    let db = db();
    let file = db.add(lone("Track.mp3", "Velo", 1));
    db.group();
    let rev = db.count(&format!(
        "SELECT rev FROM grouping_file WHERE file_id = {file}"
    ));
    db.retag(file, Some("Nightdrive"), None);
    let worked_out = WorkedOut {
        file,
        rev,
        entry: r#"[["velo"],[],[]]"#.to_string(),
    };
    db.writer.call(move |c| keep(c, &[worked_out])).unwrap();
    assert_eq!(stored(&db, file), None);
}

#[test]
fn a_stored_row_that_cant_be_read_is_worked_out_again() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.sql("UPDATE grouping_names SET entry = '{\"not\": \"three lists\"}'");
    let names = index(&db);
    assert_eq!(names.worked_out(), 1);
    assert_eq!(titles(&names, file), ["velo"]);
    assert!(stored(&db, file).unwrap().starts_with("[["));
}

#[test]
fn a_file_with_no_record_yet_is_in_the_index_and_stored_once_it_has_one() {
    let db = db();
    let file = db.add(lone("Velo.mp3", "Velo", 1));
    let names = index(&db);
    assert_eq!(titles(&names, file), ["velo"]);
    assert_eq!(stored(&db, file), None);
    db.group();
    index(&db);
    assert!(stored(&db, file).is_some());
}

#[test]
fn a_missing_file_leaves_the_index_and_coming_back_reads_its_names_again() {
    let db = db();
    let file = decided_file(&db, "Velo.mp3", "Velo", 1);
    db.set_present(file, false);
    assert_eq!(index(&db).files(), 0);
    db.set_present(file, true);
    assert_eq!(titles(&index(&db), file), ["velo"]);
}

#[test]
fn tags_that_cant_be_read_go_by_the_file_name_and_their_neighbours_go_through() {
    let db = db();
    let files = [
        db.add(lone("Velo.mp3", "Velo", 1)),
        db.add(lone("Broken.mp3", "Lost Title", 2)),
        db.add(lone("Not Text.mp3", "Lost Title", 3)),
        db.add(lone("Nightdrive.mp3", "Nightdrive", 4)),
    ];
    db.sql(&format!(
        "UPDATE file SET raw_tags = '{{\"id3v2\": 7}}' WHERE id = {};
         UPDATE file SET raw_tags = CAST(x'7B2261223A22FF227D' AS TEXT) WHERE id = {};",
        files[1], files[2]
    ));
    let (decided, _) = db.decide();
    assert_eq!(decided.files, 4, "every file of the page is decided");
    let names = db.names.lock().unwrap();
    assert_eq!(titles(&names, files[0]), ["velo"]);
    assert_eq!(titles(&names, files[1]), ["broken"]);
    assert_eq!(titles(&names, files[2]), ["not text"]);
    assert_eq!(titles(&names, files[3]), ["nightdrive"]);
    drop(names);
    assert_eq!(index(&db).worked_out(), 0, "and their keys are kept too");
}

/// `count` files with a record each and tags to parse, quickly.
fn a_library(db: &Db, count: usize) {
    db.sql(&format!(
        r#"WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
           INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
           SELECT 1, 'f' || i || '.mp3', 'f' || i || '.mp3',
                  '{{"id3v2": [{{"key": "TIT2", "value": {{"type": "text", "text": "Song ' || i || '"}}}}]}}'
           FROM n;
           INSERT INTO grouping_file (file_id) SELECT id FROM file;"#
    ));
}

#[test]
fn working_out_a_whole_library_moves_the_progress_forward_page_by_page() {
    let db = db();
    a_library(&db, 2500);
    let reads = ReadPool::with_size(db.writer.guarded_path(), 1).unwrap();
    let mut names = Names::default();
    let mut ticks = Vec::new();
    names
        .catch_up::<DbError>(
            &db.writer,
            &reads,
            &mut |fraction| {
                ticks.push(fraction);
                Ok(())
            },
            &mut || Ok(()),
        )
        .unwrap();
    assert_eq!(names.worked_out(), 2500);
    assert_eq!(ticks.len(), 3, "one report a page: {ticks:?}");
    assert!(ticks.windows(2).all(|two| two[0] < two[1]), "{ticks:?}");
    assert!(ticks[2] > 0.0 && ticks[2] < 0.3, "{ticks:?}");
}

#[test]
fn deciding_progress_keeps_rising_after_the_keys_were_worked_out() {
    let db = db();
    a_library(&db, 2500);
    let reads = ReadPool::with_size(db.writer.guarded_path(), 1).unwrap();
    let mut names = Names::default();
    let mut ticks = Vec::new();
    let share = names
        .catch_up::<DbError>(
            &db.writer,
            &reads,
            &mut |fraction| {
                ticks.push(fraction);
                Ok(())
            },
            &mut || Ok(()),
        )
        .unwrap();
    assert_eq!(share, 0.3);
    let last_of_the_fill = ticks.last().copied().unwrap();
    // The deciding loop's first tick, as the loop computes it.
    let first_of_deciding = share + (1.0 - share) * (1.0 / 2500.0);
    assert!(first_of_deciding > last_of_the_fill, "{ticks:?}");
    // With nothing to work out the whole bar is the deciding's.
    let again = names
        .catch_up::<DbError>(&db.writer, &reads, &mut |_| Ok(()), &mut || Ok(()))
        .unwrap();
    assert_eq!(again, 0.0);
}

#[test]
fn reading_stored_keys_is_no_progress_and_never_stops_between_pages() {
    let db = db();
    a_library(&db, 1500);
    index(&db);
    let reads = ReadPool::with_size(db.writer.guarded_path(), 1).unwrap();
    let mut names = Names::default();
    let (mut ticks, mut stops) = (Vec::new(), 0);
    names
        .catch_up::<DbError>(
            &db.writer,
            &reads,
            &mut |fraction| {
                ticks.push(fraction);
                Ok(())
            },
            &mut || {
                stops += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!((names.worked_out(), stops), (0, 0));
    assert_eq!(ticks, [0.0, 0.0]);
}

#[test]
fn working_out_keys_stops_between_pages_and_the_next_run_goes_on_from_there() {
    let db = db();
    a_library(&db, 2500);
    let reads = ReadPool::with_size(db.writer.guarded_path(), 1).unwrap();
    let mut names = Names::default();
    let stopped = names.catch_up::<DbError>(&db.writer, &reads, &mut |_| Ok(()), &mut || {
        Err(DbError::WriterGone)
    });
    assert!(stopped.is_err());
    assert_eq!(db.count("SELECT count(*) FROM grouping_names"), 1000);
    let next = index(&db);
    assert_eq!((next.files(), next.worked_out()), (2500, 1500));
}

#[test]
fn a_restart_with_every_key_stored_builds_the_index_without_the_writer() {
    let db = db();
    a_library(&db, 50);
    index(&db);
    // The writer is busy for as long as the index takes.
    let (entered_tx, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let writer = db.writer.clone();
    let holder = std::thread::spawn(move || {
        writer.call(move |_| {
            entered_tx.send(()).unwrap();
            Ok(released.recv_timeout(Duration::from_secs(30)).is_err())
        })
    });
    entered.recv().unwrap();
    assert_eq!(index(&db).files(), 50);
    release.send(()).unwrap();
    let waited = holder.join().unwrap().unwrap();
    assert!(!waited, "the index waited for the writer");
}

/// Keys only change with the parser, and a parser change that changes them
/// must move `RULES_VERSION`, or old rows would stand: if this fails, bump
/// `grouping::RULES_VERSION` (every file is then decided again and its keys
/// worked out again), and pin the version and the new keys here.
#[test]
fn a_change_to_what_the_keys_are_needs_a_new_rules_version() {
    let entry = |titles: &[&str], parts: &[&str], artists: &[&str]| {
        let keys = |keys: &[&str]| keys.iter().map(|key| key.to_string()).collect();
        Entry {
            titles: keys(titles),
            parts: keys(parts),
            artists: keys(artists),
        }
    };
    assert_eq!(RULES_VERSION, 3);
    assert_eq!(
        Entry::new(Some("Déjà Vu"), Some("Halden Rook"), "Déjà Vu.mp3"),
        entry(&["deja vu"], &[], &["halden rook"])
    );
    assert_eq!(
        Entry::new(
            Some("Velo (Radio Edit)"),
            Some("Halden Rook feat. Quill Ashby"),
            "03 Halden Rook - Velo.mp3"
        ),
        entry(&["velo", "velo radio edit"], &[], &["halden rook"])
    );
    assert_eq!(
        Entry::new(Some("Velo x Paper Harbor"), None, "Velo x Paper Harbor.mp3"),
        entry(&["velo x paper harbor"], &["paper harbor", "velo"], &[])
    );
    assert_eq!(Entry::new(Some(""), None, "---.mp3"), entry(&[], &[], &[]));
}

/// A snapshot of the keys of many spellings (separators, mashups, brackets,
/// features, accents, dots, numbering, empty names), as one hash. A change
/// to what any key is fails this: bump `grouping::RULES_VERSION`, then pin
/// the new hash.
#[test]
fn a_change_to_any_key_needs_a_new_rules_version() {
    let inputs: [(Option<&str>, Option<&str>, &str); 24] = [
        (Some("Déjà Vu"), Some("Halden Rook"), "Déjà Vu.mp3"),
        (
            Some("Velo (Radio Edit)"),
            Some("Halden Rook feat. Quill Ashby"),
            "03 Halden Rook - Velo.mp3",
        ),
        (Some("Velo x Paper Harbor"), None, "Velo x Paper Harbor.mp3"),
        (
            Some("Velo vs. Paper Harbor"),
            None,
            "Velo vs. Paper Harbor.mp3",
        ),
        (Some("Velo & Paper Harbor (Mashup)"), None, "a.mp3"),
        (Some("Velo / Paper Harbor"), None, "a.mp3"),
        (None, None, "Halden Rook - Velo (Extended Mix).flac"),
        (None, None, "01. Halden Rook - Velo [Remix].mp3"),
        (
            None,
            None,
            "Halden Rook, Quill Ashby - Velo ft. Mira Dale.mp3",
        ),
        (
            Some("Velo (Quill Ashby Remix)"),
            Some("Halden Rook & Quill Ashby"),
            "x.mp3",
        ),
        (Some("Velo - Original Mix"), Some("Halden Rook"), "x.mp3"),
        (
            Some("Velo [Free Download]"),
            Some("DJ Halden Rook"),
            "x.mp3",
        ),
        (Some("V.e.l.o"), Some("H. Rook"), "x.mp3"),
        (Some("VELO"), Some("HALDEN ROOK"), "x.mp3"),
        (
            Some("Velo  Paper   Harbor"),
            Some(" Halden  Rook "),
            "x.mp3",
        ),
        (Some("Ça va"), Some("Zoë Åkesson"), "x.mp3"),
        (Some("Straße"), Some("Ørjan"), "x.mp3"),
        (Some("Velo (feat. Quill Ashby)"), None, "x.mp3"),
        (Some("Velo (Halden Rook Remix) (Radio Edit)"), None, "x.mp3"),
        (Some("The Velo"), Some("The Halden Rook"), "x.mp3"),
        (Some("Velo – Paper Harbor"), None, "Velo – Paper Harbor.mp3"),
        (Some("Velo + Paper Harbor"), None, "x.mp3"),
        (Some("  "), Some(""), "folder - 12 - Velo.mp3"),
        (Some(""), None, "---.mp3"),
    ];
    // FNV-1a over the keys, in a fixed order: stable across runs and builds.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (title, artist, file) in inputs {
        for byte in format!("{:?}", Entry::new(title, artist, file)).bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    assert_eq!(RULES_VERSION, 3);
    assert_eq!(
        hash, 17_724_384_926_676_626_428,
        "the keys changed: bump RULES_VERSION, pin this hash"
    );
}
