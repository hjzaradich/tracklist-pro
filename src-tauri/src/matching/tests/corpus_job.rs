//! The grouping job over files with known answers (1bD-3).
//!
//! [`super::local_corpus`] runs the owner's real, private pairs through the
//! whole grouping job on a PC that has them. This is its made-up
//! counterpart, which CI can run: the synthetic library of [`super::corpus`],
//! given invented titles and an invented artist, goes through the same steps
//! (the quick grouping pass, the matching pass, the deciding job). Two files
//! share a track exactly when they hold the same recording: the duplicate
//! groups merge, and nothing else does.
//!
//! The helpers below are shared with the local corpus test, so both read
//! "share a track" and the stored pair the same way. They return ids, words
//! and reason codes, never anything a file's tags or name held.

use crate::grouping::tests::support::{db, raw_tags, Db, RealFile};

use super::corpus::{corpus, Corpus};

/// Whether `a` and `b` sit in one track.
pub(super) fn same_track(db: &Db, a: i64, b: i64) -> bool {
    matches!((db.track(a), db.track(b)), (Some(x), Some(y)) if x == y)
}

/// The stored pair of `a` and `b` as `outcome/state [reason codes]`, or
/// `no pair stored` where the rules said "different" and kept nothing.
///
/// Codes only: a reason's other fields can hold words from a title.
pub(super) fn stands_as(db: &Db, a: i64, b: i64) -> String {
    let Some((outcome, state)) = db.pair(a, b) else {
        return String::from("no pair stored");
    };
    let reasons: serde_json::Value =
        serde_json::from_str(&db.reasons(a, b)).expect("reasons are JSON");
    let codes: Vec<&str> = reasons
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|reason| reason["code"].as_str())
        .collect();
    format!("{outcome}/{state} [{}]", codes.join(", "))
}

/// The tag title the made-up library gives a file: its name without the
/// extension, except that the formats and encodes of one recording share a
/// title, as copies of one recording do, and so do two different songs the
/// library names alike.
fn invented_title(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if stem.starts_with("glasswing") {
        String::from("Glasswing")
    } else if stem.starts_with("core (") {
        String::from("Core")
    } else {
        stem.to_string()
    }
}

/// The file name the made-up library gives a file. The copies of one
/// recording are named alike, as a person who keeps them in separate
/// folders would: the library's own "(lead-in)" and "(320)" are note-keeping
/// of the tests, and to the version check a word in brackets can read as a
/// version.
fn invented_file_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if stem.starts_with("glasswing") => format!("Glasswing.{ext}"),
        _ => name.to_string(),
    }
}

/// The made-up library in a database, with distinct audio and whole-file
/// hashes per file (formats of one recording differ in both), and the
/// ids in `corpus.files` order.
fn loaded(corpus: &Corpus) -> (Db, Vec<i64>) {
    let db = db();
    let ids = corpus
        .files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let n = u8::try_from(i + 1).expect("a small library");
            let tags = raw_tags(Some(&invented_title(file.name)), Some("Nora Quill"));
            db.add_real(RealFile {
                rel_path: &format!("{i}/{}", invented_file_name(file.name)),
                audio: Some([n; 34]),
                blake3: [n ^ 0x80; 32],
                raw_tags: Some(&tags),
                fingerprint: &file.fingerprint,
            })
        })
        .collect();
    db.decide();
    (db, ids)
}

#[test]
fn the_job_merges_the_duplicate_groups_of_the_made_up_library() {
    let corpus = corpus();
    let (db, ids) = loaded(corpus);
    for (a, b) in corpus.duplicate_pairs() {
        let (a, b) = (ids[corpus.index_of(a)], ids[corpus.index_of(b)]);
        assert!(
            same_track(&db, a, b),
            "files {a} and {b} hold one recording but sit in two tracks: {}",
            stands_as(&db, a, b)
        );
    }
}

#[test]
fn the_job_merges_no_two_files_of_the_made_up_library_that_are_not_duplicates() {
    let corpus = corpus();
    let (db, ids) = loaded(corpus);
    let duplicates = corpus.duplicate_pairs();
    let is_duplicate = |a: &str, b: &str| {
        duplicates
            .iter()
            .any(|&(x, y)| (x == a && y == b) || (x == b && y == a))
    };
    let mut merged = Vec::new();
    for (i, a) in corpus.files.iter().enumerate() {
        for (j, b) in corpus.files.iter().enumerate().skip(i + 1) {
            if !is_duplicate(a.name, b.name) && same_track(&db, ids[i], ids[j]) {
                merged.push(format!(
                    "files {} and {}: {}",
                    ids[i],
                    ids[j],
                    stands_as(&db, ids[i], ids[j])
                ));
            }
        }
    }
    assert!(
        merged.is_empty(),
        "files that are not duplicates share a track:\n{}",
        merged.join("\n")
    );
}
