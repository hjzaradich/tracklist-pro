//! The deciding run and the grouping pass over what it stores, through the
//! database: made-up files with names, audio hashes and fingerprints, the
//! real matching pass, the real rules.

use super::support::*;
use crate::grouping::{decide, Decided, MAX_TITLE_BUCKET};

/// About two minutes of made-up audio.
const LEN: usize = 900;

fn file<'a>(name: &'a str, title: &'a str, n: u8, items: Vec<u32>) -> NewFile<'a> {
    NewFile {
        name,
        title: Some(title),
        artist: Some("Halden Rook"),
        hash: Some(hash(n)),
        items: Some(items),
    }
}

/// A file with a name and its own audio, and no fingerprint.
fn named<'a>(name: &'a str, title: &'a str, artist: Option<&'a str>, n: u8) -> NewFile<'a> {
    NewFile {
        name,
        title: Some(title),
        artist,
        hash: Some(hash(n)),
        items: None,
    }
}

// ---- duplicates -----------------------------------------------------------------

#[test]
fn a_reencode_of_the_same_track_joins_it() {
    let db = db();
    let velo = track(1, LEN);
    let flac = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let mp3 = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    let other = db.add(file("Nightdrive.flac", "Nightdrive", 3, track(2, LEN)));
    db.group();
    assert_eq!(
        db.tracks(),
        3,
        "different audio hashes: three tracks so far"
    );
    let (decided, summary) = db.decide();
    assert_eq!(db.track(flac), db.track(mp3));
    assert_ne!(db.track(flac), db.track(other));
    assert_eq!(db.tracks(), 2);
    assert_eq!(db.stands(flac, mp3), ("duplicate".into(), "merged".into()));
    assert!(db.reasons(flac, mp3).contains("sameAudio"));
    assert_eq!((decided.files, summary.moved), (3, 1));
}

#[test]
fn the_same_audio_under_two_cuts_names_is_linked_never_merged() {
    // Clean and Dirty fingerprint alike: the version check's veto.
    let db = db();
    let velo = track(1, LEN);
    let clean = db.add(file("Velo (Clean).mp3", "Velo (Clean)", 1, velo.clone()));
    let dirty = db.add(file(
        "Velo (Dirty).mp3",
        "Velo (Dirty)",
        2,
        reencoded(&velo, 1, 7),
    ));
    db.decide();
    assert_ne!(db.track(clean), db.track(dirty));
    assert_eq!(db.stands(clean, dirty), ("cut".into(), "linked".into()));
    let (kind, label, source, _) = db.link(clean, dirty).expect("a version link");
    assert_eq!((kind.as_str(), source.as_str()), ("cut", "fingerprint"));
    assert_eq!(label.as_deref(), Some("clean/dirty"));
}

#[test]
fn a_radio_edit_found_inside_the_full_track_gets_a_cut_link() {
    let db = db();
    let velo = track(1, LEN);
    let full = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let edit = db.add(file(
        "Velo (Radio Edit).flac",
        "Velo (Radio Edit)",
        2,
        reencoded(&velo[300..700], 3, 7),
    ));
    db.decide();
    assert_ne!(db.track(full), db.track(edit));
    let (kind, _, source, _) = db.link(full, edit).expect("a version link");
    assert_eq!((kind.as_str(), source.as_str()), ("cut", "fingerprint"));
    assert!(db.reasons(full, edit).contains("audioInsideOther"));
}

#[test]
fn a_remix_the_names_tell_apart_gets_a_rework_link_by_the_names_alone() {
    let db = db();
    let original = db.add(file("Velo.flac", "Velo", 1, track(1, LEN)));
    let remix = db.add(file(
        "Velo (Quill Ashby Remix).flac",
        "Velo (Quill Ashby Remix)",
        2,
        track(2, LEN),
    ));
    db.decide();
    assert_ne!(db.track(original), db.track(remix));
    let (kind, _, source, _) = db.link(original, remix).expect("a version link");
    assert_eq!((kind.as_str(), source.as_str()), ("rework", "parser"));
}

// ---- review -------------------------------------------------------------------------

#[test]
fn a_pair_the_rules_send_to_review_stays_apart_and_is_stored_with_its_reasons() {
    // Identical tags can hide a different recording: half the audio is
    // shared, the names are the same.
    let db = db();
    let velo = track(1, LEN);
    let other_take = [&velo[..LEN / 2], &track(9, LEN / 2)[..]].concat();
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, reencoded(&other_take, 4, 7)));
    let (_, summary) = db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.tracks(), 2);
    assert_eq!(db.stands(a, b), ("review".into(), "open".into()));
    assert!(db.reasons(a, b).contains("sameTagsPartialAudio"));
    assert_eq!(db.link(a, b), None, "a review pair is not a version");
    assert_eq!(summary.moved, 0);
    // And it stays that way.
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.stands(a, b), ("review".into(), "open".into()));
}

#[test]
fn the_same_names_with_no_fingerprint_to_check_them_against_go_to_review_not_to_one_track() {
    let db = db();
    let a = db.add(named("Velo.mp3", "Velo", Some("Halden Rook"), 1));
    let b = db.add(named("copy/Velo.mp3", "Velo", Some("Halden Rook"), 2));
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.stands(a, b), ("review".into(), "open".into()));
    assert!(db.reasons(a, b).contains("noFingerprint"));
}

// ---- pairs the matching pass never proposed -----------------------------------------

#[test]
fn a_same_named_pair_the_pass_never_proposed_is_compared_directly_and_merges() {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, unproposed(&velo, 5)));
    db.group();
    db.compare_fingerprints();
    assert!(
        !db.compared(a, b),
        "the pass has nothing to propose the pair by"
    );
    let (decided, _) = db.decide();
    assert_eq!(decided.compared_directly, 1);
    assert!(db.compared(a, b), "the comparison is kept");
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(db.stands(a, b), ("duplicate".into(), "merged".into()));
    // The stored comparison does for the next run: nothing is compared again.
    db.retag(b, Some("Velo"), Some("Halden Rook feat. Ivo Marsh"));
    let (again, _) = db.decide();
    assert_eq!((again.files, again.compared_directly), (1, 0));
}

#[test]
fn a_pair_with_other_names_that_the_pass_never_proposed_is_not_compared() {
    // Nothing is compared across the whole library: only where the names
    // call for it.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file(
        "Nightdrive.mp3",
        "Nightdrive",
        2,
        unproposed(&velo, 5),
    ));
    let (decided, _) = db.decide();
    assert_eq!(decided.compared_directly, 0);
    assert!(!db.compared(a, b));
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.pair(a, b), None, "never asked about, nothing stored");
}

#[test]
fn a_mashup_gets_a_link_to_the_part_it_names_and_shares_audio_with() {
    let db = db();
    let velo = track(1, LEN);
    // The mashup holds half of "Velo" and nothing of "Nightdrive".
    let mixed = [&unproposed(&velo, 6)[..LEN / 2], &track(8, LEN / 2)[..]].concat();
    let part = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let named_only = db.add(file("Nightdrive.flac", "Nightdrive", 2, track(2, LEN)));
    let mashup = db.add(file("Velo x Nightdrive.mp3", "Velo x Nightdrive", 3, mixed));
    db.group();
    db.compare_fingerprints();
    assert!(!db.compared(part, mashup));
    let (decided, _) = db.decide();
    // Both parts it names are compared; only one shares audio.
    assert_eq!(decided.compared_directly, 2);
    assert_eq!(db.tracks(), 3, "a mashup is never merged with its part");
    let (kind, label, source, mashup_first) = db.link(mashup, part).expect("a version link");
    assert_eq!((kind.as_str(), source.as_str()), ("rework", "fingerprint"));
    assert_eq!(label.as_deref(), Some("mashup-contains"));
    assert!(mashup_first, "the mashup contains its part");
    assert!(db.reasons(part, mashup).contains("mashupContains"));
    assert_eq!(db.link(mashup, named_only), None);
    assert_eq!(db.pair(mashup, named_only), None);
}

#[test]
fn a_pair_inside_a_set_that_nobody_proposed_is_decided_before_the_set_merges() {
    // 1 and 2 are proposed, 2 and 3 are proposed; 1 and 3 share no item bit
    // for bit and have other names, so nothing proposes them.
    let db = db();
    let one = track(1, LEN);
    let two = reencoded(&one, 1, 7);
    // Where 2 kept 1's item, 3 differs from both; elsewhere 3 is 2.
    let three: Vec<u32> = one
        .iter()
        .zip(&two)
        .map(|(a, b)| if a == b { a ^ 0b101 } else { *b })
        .collect();
    let a = db.add(file("Velo.flac", "Velo", 1, one));
    let b = db.add(file("Velo.mp3", "Velo", 2, two));
    let c = db.add(file("Nightdrive.mp3", "Nightdrive", 3, three));
    db.group();
    db.compare_fingerprints();
    assert!(db.compared(a, b) && db.compared(b, c) && !db.compared(a, c));
    let (decided, _) = db.decide();
    assert_eq!((decided.completed, decided.compared_directly), (1, 1));
    assert!(db.compared(a, c));
    assert_eq!(db.tracks(), 1, "every pair is a duplicate: one track");
    assert_eq!(db.stands(a, c), ("duplicate".into(), "merged".into()));
}

#[test]
fn a_set_whose_pairs_disagree_merges_nothing_and_its_pairs_are_stored_as_held() {
    // Three encodes, each made from the one before: 1 and 2 are
    // duplicates, 2 and 3 are duplicates, but 1 and 3 differ by more than
    // the duplicate rule allows.
    let db = db();
    let first = track(1, LEN);
    let second = worn(&first, 1, 7, 5);
    let third = worn(&second, 2, 7, 5);
    let a = db.add(file("Velo.flac", "Velo", 1, first));
    let b = db.add(file("Velo.m4a", "Velo", 2, second));
    let c = db.add(file("Velo.mp3", "Velo", 3, third));
    let (_, summary) = db.decide();
    assert_eq!(db.stands(a, b).0, "duplicate");
    assert_eq!(db.stands(b, c).0, "duplicate");
    assert_eq!(db.stands(a, c), ("review".into(), "open".into()));
    assert_eq!(db.tracks(), 3, "nothing in the set merges");
    assert_eq!(summary.sets_disagreeing, 1);
    assert_eq!(db.stands(a, b).1, "set_disagrees");
    assert_eq!(db.stands(b, c).1, "set_disagrees");
    // Had the third arrived after the first two merged, they would have
    // stayed one track: merges are kept.
    let later = super::support::db();
    let first = track(1, LEN);
    let second = worn(&first, 1, 7, 5);
    let third = worn(&second, 2, 7, 5);
    let a = later.add(file("Velo.flac", "Velo", 1, first));
    let b = later.add(file("Velo.m4a", "Velo", 2, second));
    later.decide();
    assert_eq!(later.track(a), later.track(b));
    let c = later.add(file("Velo.mp3", "Velo", 3, third));
    later.decide();
    assert_eq!(later.track(a), later.track(b), "the track stays whole");
    assert_ne!(later.track(c), later.track(a), "the newcomer stays apart");
    assert_eq!(later.stands(b, c).1, "set_disagrees");
}

// ---- the Library ---------------------------------------------------------------------------

#[test]
fn two_library_tracks_with_identical_audio_stay_two_tracks_and_the_pair_is_stored() {
    let db = db();
    let a = db.add(named("Velo.mp3", "Velo", None, 1));
    let b = db.add(named("gigs/Velo.mp3", "Velo", None, 2));
    db.group();
    let (ta, tb) = (db.to_library(a), db.to_library(b));
    // rekordbox had two entries for one recording: the very same audio.
    db.set_hash(b, Some(hash(1)));
    let (_, summary) = db.decide();
    assert_eq!((db.track(a), db.track(b)), (Some(ta), Some(tb)));
    assert_eq!(db.tracks(), 2);
    assert_eq!(
        db.stands(a, b),
        ("duplicate".into(), "both_in_library".into())
    );
    assert!(db.reasons(a, b).contains("identicalAudio"));
    assert_eq!(summary.sets_both_in_library, 1);
    assert_eq!(
        db.count("SELECT count(*) FROM library_track"),
        2,
        "no Library track is merged away"
    );
}

#[test]
fn two_library_tracks_the_fingerprints_call_duplicates_stay_two_tracks_too() {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.group();
    db.to_library(a);
    db.to_library(b);
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(
        db.stands(a, b),
        ("duplicate".into(), "both_in_library".into())
    );
}

#[test]
fn a_file_a_library_track_points_at_never_moves_and_a_duplicate_outside_the_library_joins_it() {
    let db = db();
    let velo = track(1, LEN);
    let outside = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let pinned = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.group();
    let library_track = db.to_library(pinned);
    assert!(db.track(outside).unwrap() < library_track, "the lower id");
    db.decide();
    assert_eq!(db.track(pinned), Some(library_track), "it never moves");
    assert_eq!(
        db.track(outside),
        Some(library_track),
        "an extra file of it"
    );
    assert_eq!(
        db.count("SELECT linked_file_id FROM library_track"),
        pinned,
        "the Library track still plays its own file"
    );
}

// ---- the same again ---------------------------------------------------------------------------

/// A small library with one of each: a re-encode, two cuts' names, a cut
/// by audio, a review pair, a mashup, a remix by name, two Library copies.
fn a_bit_of_everything(db: &Db) {
    let (velo, drive) = (track(1, LEN), track(2, LEN));
    db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.add(file(
        "Velo (Clean).mp3",
        "Velo (Clean)",
        3,
        reencoded(&velo, 2, 7),
    ));
    db.add(file(
        "Velo (Radio Edit).mp3",
        "Velo (Radio Edit)",
        4,
        reencoded(&velo[300..700], 3, 7),
    ));
    db.add(file("Nightdrive.flac", "Nightdrive", 5, drive.clone()));
    let other_take = [&drive[..LEN / 2], &track(9, LEN / 2)[..]].concat();
    db.add(file("Nightdrive.mp3", "Nightdrive", 6, other_take));
    let mixed = [&unproposed(&velo, 6)[..LEN / 2], &track(8, LEN / 2)[..]].concat();
    db.add(file(
        "Velo x Paper Harbor.mp3",
        "Velo x Paper Harbor",
        7,
        mixed,
    ));
    db.add(file(
        "Nightdrive (Quill Ashby Remix).mp3",
        "Nightdrive (Quill Ashby Remix)",
        8,
        track(3, LEN),
    ));
    let lone = db.add(named("Paper Harbor.mp3", "Paper Harbor", None, 9));
    let twin = db.add(named("usb/Paper Harbor.mp3", "Paper Harbor", None, 10));
    db.group();
    db.to_library(lone);
    db.to_library(twin);
    db.set_hash(twin, Some(hash(9)));
}

#[test]
fn a_run_over_an_unchanged_database_changes_nothing() {
    let db = db();
    a_bit_of_everything(&db);
    let (first, _) = db.decide();
    assert!(first.files > 0 && first.recorded > 0);
    let before = db.dump();
    assert!(before.contains("link ") && before.contains(" open ") && before.contains(" merged "));
    let (again, summary) = db.decide();
    assert_eq!(
        again,
        Decided::default(),
        "nothing was due, asked about or compared"
    );
    assert!(!summary.changed_tracks());
    assert_eq!((summary.links_made, summary.links_removed), (0, 0));
    assert_eq!(db.dump(), before, "not a row was written");
    // And with nothing kept in memory from the run before, as after a
    // restart.
    *db.names.lock().unwrap() = Default::default();
    assert_eq!(db.decide().0, Decided::default());
    assert_eq!(db.dump(), before);
}

#[test]
fn the_index_of_names_is_built_only_by_a_run_that_has_a_file_to_decide() {
    let db = db();
    a_bit_of_everything(&db);
    db.decide();
    let all = db.count("SELECT count(*) FROM file") as usize;
    assert_eq!(db.names.lock().unwrap().files(), all);
    // After a restart nothing is kept. A run with nothing due reads no
    // names; one new file, and the next run reads them all.
    *db.names.lock().unwrap() = Default::default();
    db.decide();
    assert_eq!(db.names.lock().unwrap().files(), 0);
    db.add(file("late/Velo.mp3", "Velo", 12, track(55, LEN)));
    let (decided, _) = db.decide();
    assert_eq!(decided.files, 1);
    assert_eq!(db.names.lock().unwrap().files(), all + 1);
}

#[test]
fn a_new_file_joins_its_track_without_moving_any_other_file() {
    let db = db();
    a_bit_of_everything(&db);
    db.decide();
    let before = db.groups();
    let velo = track(1, LEN);
    let new = db.add(file("backup/Velo.mp3", "Velo", 11, reencoded(&velo, 7, 7)));
    let (decided, summary) = db.decide();
    assert_eq!(decided.files, 1, "only the new file is decided");
    assert_eq!((summary.placed, summary.moved), (1, 1));
    assert_eq!(db.track(new), db.track(1), "with the two files of Velo");
    let mut expected = before;
    for group in &mut expected {
        if group.contains(&1) {
            group.push(new);
        }
    }
    assert_eq!(db.groups(), expected, "every other file is where it was");
}

#[test]
fn a_changed_fingerprint_has_only_that_files_pairs_decided_again() {
    let db = db();
    a_bit_of_everything(&db);
    db.decide();
    let before = db.groups();
    // The re-encode of Velo is replaced by other audio altogether.
    db.set_hash(2, Some(hash(42)));
    db.set_items(2, track(77, LEN));
    assert_eq!(db.pair(1, 2), None, "its pairs went with its fingerprint");
    let (decided, _) = db.decide();
    assert_eq!(decided.files, 1, "only the changed file is decided");
    // It still carries the title "Velo", and the pass has nothing to
    // propose it by: it's compared directly with the other file of that
    // title and with the mashup naming it as a part, and shares nothing
    // with either.
    assert_eq!(decided.compared_directly, 2);
    assert_ne!(db.track(2), db.track(1), "other audio: it left its track");
    let mut expected: Vec<Vec<i64>> = before
        .into_iter()
        .map(|group| group.into_iter().filter(|&f| f != 2).collect())
        .collect();
    expected.push(vec![2]);
    expected.sort();
    assert_eq!(db.groups(), expected, "every other file is where it was");
    // The same title over audio that shares nothing: different, so nothing
    // is stored (the fingerprint's veto).
    assert_eq!(db.pair(1, 2), None);
}

#[test]
fn a_retag_never_splits_a_track_the_pair_is_stored_as_no_longer_a_duplicate() {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.decide();
    assert_eq!(db.track(a), db.track(b));
    // The second file turns out to be labeled as the Dirty cut.
    db.retag(b, Some("Velo (Dirty)"), Some("Halden Rook"));
    let (decided, summary) = db.decide();
    assert_eq!(decided.files, 1);
    assert_eq!(db.track(a), db.track(b), "the track stays whole");
    assert_eq!(summary.moved, 0);
    assert_eq!(db.stands(a, b).1, "in_one_track");
    assert_ne!(db.stands(a, b).0, "duplicate");
    assert_eq!(
        db.count("SELECT count(*) FROM version_link"),
        0,
        "a track is not linked to itself"
    );
}

#[test]
fn files_decided_under_other_rules_are_all_due_again() {
    let db = db();
    a_bit_of_everything(&db);
    db.decide();
    let all = db.count("SELECT count(*) FROM file");
    assert_eq!(db.decided().len() as i64, all);
    // As a build with another rules version would find them.
    db.sql("UPDATE grouping_file SET decided_as = decided_as + 65536");
    assert!(db.decided().is_empty());
    let (decided, summary) = db.decide();
    assert_eq!(decided.files as i64, all);
    assert!(
        !summary.changed_tracks(),
        "the same rules give the same tracks"
    );
}

// ---- a run that stops ------------------------------------------------------------------------

/// Why a test's run stopped.
#[derive(Debug)]
enum Stopped {
    /// Where the test asked it to.
    Here,
    /// Shown when the run stopped for another reason.
    Db(#[allow(dead_code)] crate::db::DbError),
}

impl From<crate::db::DbError> for Stopped {
    fn from(e: crate::db::DbError) -> Stopped {
        Stopped::Db(e)
    }
}

#[test]
fn a_run_that_stops_leaves_every_file_decided_with_all_its_pairs_or_due() {
    let db = db();
    a_bit_of_everything(&db);
    db.group();
    db.compare_fingerprints();
    // One file a page, and the run is stopped where it asks, after the
    // first page.
    let stopped = {
        let mut names = db.names.lock().unwrap();
        decide::run::<Stopped>(&db.writer, &mut names, 1, &mut |_| Ok(()), &mut || {
            Err(Stopped::Here)
        })
    };
    assert!(matches!(stopped, Err(Stopped::Here)), "{stopped:?}");
    assert_eq!(db.decided(), [1], "only the first file is marked");
    // All of that file's pairs are there already.
    let of_first = db.count("SELECT count(*) FROM grouping_pair WHERE file_a = 1 OR file_b = 1");
    assert!(of_first >= 3, "{of_first} pairs of file 1");
    // The tracks stand as they were: nothing half done.
    assert_eq!(
        db.count(
            "SELECT count(*) FROM file f WHERE present = 1 AND
             (SELECT count(*) FROM recording_file WHERE file_id = f.id) <> 1"
        ),
        0
    );
    // The next run goes on from there, and ends where an unbroken run does.
    let (rest, _) = db.decide();
    assert_eq!(rest.files as i64, db.count("SELECT count(*) FROM file") - 1);
    assert_eq!(
        db.count("SELECT count(*) FROM grouping_pair WHERE file_a = 1 OR file_b = 1"),
        of_first
    );
    let whole = super::support::db();
    a_bit_of_everything(&whole);
    whole.decide();
    assert_eq!(db.groups(), whole.groups());
}

#[test]
fn a_file_that_changes_while_it_is_being_decided_stays_due() {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.group();
    db.compare_fingerprints();
    // The file's record moves on under the run: as if its tags were
    // rewritten after the run listed it.
    db.sql(
        "CREATE TRIGGER retagged_meanwhile AFTER INSERT ON grouping_pair
         BEGIN UPDATE grouping_file SET rev = rev + 1 WHERE file_id = 1; END",
    );
    let decided = db.decide_in_pages(10);
    assert_eq!(decided.files, 1, "the other file is decided");
    assert_eq!(db.decided(), [2]);
    db.sql("DROP TRIGGER retagged_meanwhile");
    db.decide();
    assert_eq!(db.decided(), [a, 2]);
}

// ---- titles too many files share -----------------------------------------------------------------

/// `n` files titled "Intro", with no fingerprint and their own audio each.
fn intros(db: &Db, n: usize, artist: impl Fn(usize) -> Option<String>) {
    for i in 0..n {
        let name = format!("{i:03}/Intro.mp3");
        let artist = artist(i);
        db.add(NewFile {
            name: &name,
            title: Some("Intro"),
            artist: artist.as_deref(),
            hash: Some(hash(i as u8 + 1)),
            items: None,
        });
    }
}

#[test]
fn a_title_shared_by_as_many_files_as_the_cap_has_every_pair_asked_about() {
    let db = db();
    intros(&db, MAX_TITLE_BUCKET, |_| None);
    let (decided, _) = db.decide();
    let pairs = (MAX_TITLE_BUCKET * (MAX_TITLE_BUCKET - 1) / 2) as u64;
    assert_eq!((decided.skipped_buckets, decided.skipped_pairs), (0, 0));
    assert_eq!((decided.pairs, decided.recorded), (pairs, pairs));
    assert_eq!(db.stands(1, 2), ("review".into(), "open".into()));
}

#[test]
fn a_title_shared_by_one_file_more_than_the_cap_is_skipped_and_counted() {
    let db = db();
    intros(&db, MAX_TITLE_BUCKET + 1, |_| None);
    let (decided, _) = db.decide();
    let n = MAX_TITLE_BUCKET as u64 + 1;
    assert_eq!(
        (decided.skipped_buckets, decided.skipped_pairs),
        (1, n * (n - 1) / 2)
    );
    assert_eq!((decided.pairs, decided.recorded), (0, 0));
    assert_eq!(db.count("SELECT count(*) FROM grouping_pair"), 0);
    assert_eq!(db.tracks() as u64, n, "skipped pairs stay apart");
    assert_eq!(decided.files, n, "the files are decided all the same");
}

#[test]
fn an_over_full_title_is_narrowed_to_the_files_that_share_an_artist() {
    // Two of the files are by one artist; every other has its own.
    let db = db();
    intros(&db, MAX_TITLE_BUCKET + 1, |i| match i {
        0 | 1 => Some("Halden Rook".into()),
        i => Some(format!("Artist {i}")),
    });
    let (decided, _) = db.decide();
    assert_eq!((decided.skipped_buckets, decided.skipped_pairs), (0, 0));
    assert_eq!((decided.pairs, decided.recorded), (1, 1));
    assert_eq!(db.stands(1, 2), ("review".into(), "open".into()));
}

#[test]
fn a_title_with_no_letter_or_digit_brings_no_files_together() {
    let db = db();
    let a = db.add(named("one/???.mp3", "???", None, 1));
    let b = db.add(named("two/???.mp3", "???", None, 2));
    let (decided, _) = db.decide();
    assert_eq!(decided.pairs, 0);
    assert_eq!(db.pair(a, b), None);
}
