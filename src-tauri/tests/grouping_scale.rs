//! 1bC-1: what the grouping job costs on a made-up library of 100,000
//! files, and that nothing in it grows with the square of the library.
//!
//! The files are made up in code (no audio, no real music, made-up names):
//! each has a fingerprint of about 75 seconds, an audio hash, a title and an
//! artist. Of every 100 tracks:
//!
//! - three have a re-encoded copy under the same title (duplicates the
//!   matching pass proposes);
//! - one has a cut under the title with "(Radio Edit)";
//! - one has an exact copy (the same audio hash);
//! - one has a same-named copy that shares no fingerprint item bit for bit,
//!   so the pass never proposes it and the deciding run compares it
//!   directly;
//! - one comes in three encodes, each made from the one before: the first
//!   and the second are duplicates, the second and the third too, but the
//!   first and the third differ too much, so the set's pairs disagree.
//!
//! And one title, "Intro", is shared by 300 files that name no artist: far
//! over what the deciding run compares pair by pair.
//!
//! The measurement is an ignored test. Run it alone, in a release build:
//!
//! ```text
//! cargo test --release --test grouping_scale -- --ignored --nocapture --exact a_library_of_100k
//! ```
//!
//! It prints the times; it asserts none. **`a_small_library_goes_through_the_same_steps`**
//! is the smoke test that runs in CI: the same harness on 2,000 files,
//! checking what came out rather than how long it took.

use std::time::{Duration, Instant};

use rusqlite::params;
use rusty_chromaprint::FingerprintCompressor;
use tracklist_pro_lib::db::{DbError, Writer, DB_FILE_NAME};
use tracklist_pro_lib::fingerprint::stored;
use tracklist_pro_lib::grouping::{self, Decided, Names, Summary};
use tracklist_pro_lib::matching::Matcher;
use tracklist_pro_lib::write_guard::WriteGuard;

/// splitmix64: a seeded, platform-independent random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The items of made-up track `seed`: each differs from the one before by
/// one to three bits, as neighbouring items of real audio do.
fn track(seed: u64, len: usize) -> Vec<u32> {
    let mut rng = Rng(seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ 0x7AC4);
    let mut item = rng.next() as u32;
    (0..len)
        .map(|_| {
            for _ in 0..=rng.below(3) {
                item ^= 1 << rng.below(32);
            }
            item
        })
        .collect()
}

/// `items` as another encoding would give them: one item in `keep` stays
/// bit-identical and the rest get 1 to `most` bits flipped.
fn worn(items: &[u32], seed: u64, keep: u64, most: u64) -> Vec<u32> {
    let mut rng = Rng(seed ^ 0xE4C0);
    items
        .iter()
        .map(|&item| {
            if rng.below(keep) == 0 {
                return item;
            }
            let mut noisy = item;
            for _ in 0..=rng.below(most) {
                noisy ^= 1 << rng.below(32);
            }
            noisy
        })
        .collect()
}

/// A re-encode the matching pass proposes: about 1 item in 7 identical.
fn reencoded(items: &[u32], seed: u64) -> Vec<u32> {
    worn(items, seed, 7, 4)
}

/// The same audio with one bit flipped in every item, so not one item is
/// the same bit for bit: nothing for the pass's index to propose it by.
fn unproposed(items: &[u32], seed: u64) -> Vec<u32> {
    let mut rng = Rng(seed ^ 0x51DE);
    items
        .iter()
        .map(|item| item ^ (1 << rng.below(32)))
        .collect()
}

/// The bytes `file.fingerprint` would hold for `items`.
fn blob(items: &[u32]) -> Vec<u8> {
    let config = stored::config();
    let mut blob = b"TLFP".to_vec();
    blob.push(stored::VERSION);
    blob.extend_from_slice(&FingerprintCompressor::from(&config).compress(items));
    blob
}

/// One made-up file.
struct Made {
    name: String,
    title: String,
    artist: Option<String>,
    /// What its audio hash is made from: equal for exact copies.
    audio: u64,
    items: Vec<u32>,
}

/// What was planted in a library, as indexes into its files.
#[derive(Default)]
struct Planted {
    /// Tracks with no other file like them.
    singles: Vec<usize>,
    /// Re-encodes under the same title, and exact copies: one track each.
    duplicates: Vec<(usize, usize)>,
    /// Same-named copies the pass can't propose.
    unproposed: Vec<(usize, usize)>,
    /// A full track and its Radio Edit.
    cuts: Vec<(usize, usize)>,
    /// Three encodes in a row, the first and the last too far apart:
    /// nothing merges.
    disagreeing: Vec<[usize; 3]>,
}

/// How many files share the title "Intro".
const INTROS: usize = 300;

/// A library being made up.
#[derive(Default)]
struct Making {
    files: Vec<Made>,
    next_audio: u64,
}

impl Making {
    /// Adds a file of track `seed` whose title ends in `tail`. `audio` is
    /// the audio of an exact copy, or `None` for audio of its own. Returns
    /// its index and its audio.
    fn add(&mut self, seed: u64, tail: &str, audio: Option<u64>, items: Vec<u32>) -> (usize, u64) {
        self.next_audio += 1;
        let audio = audio.unwrap_or(self.next_audio);
        let (artist, title) = (
            format!("Artist {}", seed % 997),
            format!("Song {seed}{tail}"),
        );
        self.files.push(Made {
            // Its own folder each, so copies can share a file name.
            name: format!("{:07}/{artist} - {title}.mp3", self.files.len()),
            title,
            artist: Some(artist),
            audio,
            items,
        });
        (self.files.len() - 1, audio)
    }
}

/// A library of about `n` files, `len` items a fingerprint.
fn library(n: usize, len: usize) -> (Vec<Made>, Planted) {
    let mut making = Making::default();
    let mut planted = Planted::default();
    let mut seed = 0u64;
    let intros = INTROS.min(n / 4);
    while making.files.len() < n - intros {
        seed += 1;
        let items = track(seed, len);
        let (original, audio) = making.add(seed, "", None, items.clone());
        match seed % 100 {
            1..=3 => {
                let (copy, _) = making.add(seed, "", None, reencoded(&items, seed));
                planted.duplicates.push((original, copy));
            }
            4 => {
                let edit = reencoded(&items[len / 4..len * 3 / 4], seed);
                let (cut, _) = making.add(seed, " (Radio Edit)", None, edit);
                planted.cuts.push((original, cut));
            }
            5 => {
                let (copy, _) = making.add(seed, "", Some(audio), items.clone());
                planted.duplicates.push((original, copy));
            }
            6 => {
                let (copy, _) = making.add(seed, "", None, unproposed(&items, seed));
                planted.unproposed.push((original, copy));
            }
            7 => {
                // Each a rough encode of the one before: about 2.6 bits an
                // item apart, and the first and the third about 5.
                let second = worn(&items, seed, 7, 5);
                let third = worn(&second, seed ^ 0x77, 7, 5);
                let (second, _) = making.add(seed, "", None, second);
                let (third, _) = making.add(seed, "", None, third);
                planted.disagreeing.push([original, second, third]);
            }
            _ => planted.singles.push(original),
        }
    }
    let mut files = making.files;
    for i in 0..intros {
        files.push(Made {
            name: format!("{:07}/Intro.mp3", files.len()),
            title: "Intro".into(),
            artist: None,
            audio: u64::MAX / 8 + i as u64,
            items: track(u64::MAX / 2 + i as u64, len),
        });
    }
    (files, planted)
}

/// A migrated database in a temp folder with one music folder.
struct Db {
    writer: Writer,
    _dir: tempfile::TempDir,
}

impl Db {
    fn new() -> Db {
        let dir = tempfile::Builder::new()
            .prefix("tlp-grouping-")
            .tempdir()
            .unwrap();
        let guard = WriteGuard::app_data(dir.path()).unwrap();
        let path = guard.check(&dir.path().join(DB_FILE_NAME)).unwrap();
        let writer = Writer::open(&path).unwrap();
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
                )
            })
            .unwrap();
        Db { writer, _dir: dir }
    }

    /// Adds a present file for each of `files`, as the read, hash and
    /// fingerprint stages leave them. Returns their ids, in order.
    fn add(&self, files: &[Made]) -> Vec<i64> {
        let mut ids = Vec::with_capacity(files.len());
        for chunk in files.chunks(1000) {
            let rows: Vec<(String, Vec<u8>, String, Vec<u8>)> = chunk
                .iter()
                .map(|file| {
                    let frame = |key: &str, text: &str| {
                        serde_json::json!({ "key": key, "value": { "type": "text", "text": text } })
                    };
                    let mut frames = vec![frame("TIT2", &file.title)];
                    frames.extend(file.artist.as_deref().map(|artist| frame("TPE1", artist)));
                    let mut hash = vec![1u8; 34];
                    hash[2..10].copy_from_slice(&file.audio.to_be_bytes());
                    (
                        file.name.clone(),
                        hash,
                        serde_json::json!({ "id3v2": frames }).to_string(),
                        blob(&file.items),
                    )
                })
                .collect();
            let added: Vec<i64> = self
                .writer
                .call(move |c| {
                    let tx = c.transaction()?;
                    let mut ids = Vec::new();
                    for (name, hash, tags, blob) in &rows {
                        tx.execute(
                            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime,
                                               audio_hash, raw_tags, fingerprint)
                             VALUES (1, ?1, ?1, 1000, 1000, ?2, ?3, ?4)",
                            params![name, hash, tags, blob],
                        )?;
                        ids.push(tx.last_insert_rowid());
                    }
                    tx.commit()?;
                    Ok(ids)
                })
                .unwrap();
            ids.extend(added);
        }
        ids
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .unwrap()
    }

    fn track_of(&self, file: i64) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT recording_id FROM recording_file WHERE file_id = ?1",
                    [file],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn linked(&self, a: i64, b: i64) -> bool {
        let (ta, tb) = (self.track_of(a), self.track_of(b));
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM version_link
                                    WHERE (recording_a = ?1 AND recording_b = ?2)
                                       OR (recording_a = ?2 AND recording_b = ?1))",
                    [ta, tb],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = work();
    (out, start.elapsed())
}

fn group(db: &Db) -> (Summary, Duration) {
    timed(|| db.writer.call(grouping::regroup).unwrap())
}

fn compare_fingerprints(matcher: &Matcher, db: &Db) -> Duration {
    timed(|| {
        matcher
            .pass::<DbError>(&db.writer, &mut |_| Ok(()))
            .unwrap()
    })
    .1
}

fn decide(names: &mut Names, db: &Db) -> (Decided, Duration) {
    timed(|| {
        grouping::decide::run::<DbError>(
            &db.writer,
            names,
            grouping::PAGE,
            &mut |_| Ok(()),
            &mut || Ok(()),
        )
        .unwrap()
    })
}

/// What the job's steps did on one library, and how long each took.
struct Report {
    files: usize,
    planted: Planted,
    ids: Vec<i64>,
    /// The quick job: every file gets a track, exact copies together.
    placing: (Summary, Duration),
    comparing: Duration,
    /// The deciding job: the deciding run, then a grouping pass.
    first: (Decided, Duration),
    after_first: (Summary, Duration),
    /// The same after 100 more files arrive: 50 re-encodes, 50 new tracks.
    new_ids: Vec<i64>,
    later: (Decided, Duration),
    after_later: (Summary, Duration),
    /// With nothing due: the index of names kept, and built afresh as after
    /// a restart.
    idle: (Decided, Duration),
    restarted: (Decided, Duration),
    /// How many files' names the run after a restart read with nothing due.
    restarted_names: usize,
    /// One new file after a restart: the index of names is built again.
    restarted_one: (Decided, Duration),
    idle_pass: (Summary, Duration),
    db: Db,
}

fn steps(n: usize, len: usize) -> Report {
    let (files, planted) = library(n, len);
    let db = Db::new();
    let ids = db.add(&files);
    let matcher = Matcher::new();
    let mut names = Names::default();

    let placing = group(&db);
    let comparing = compare_fingerprints(&matcher, &db);
    let first = decide(&mut names, &db);
    let after_first = group(&db);

    // 100 more files: re-encodes of 50 tracks that had no copy, under
    // their titles, and 50 tracks nobody has.
    let mut new = Vec::new();
    for (i, &single) in planted.singles.iter().take(50).enumerate() {
        let file = &files[single];
        new.push(Made {
            name: format!("new/{i:03}/{}.mp3", file.title),
            title: file.title.clone(),
            artist: file.artist.clone(),
            audio: u64::MAX / 2 + i as u64,
            items: reencoded(&file.items, 0xABCD + i as u64),
        });
    }
    for i in 0..50u64 {
        new.push(Made {
            name: format!("new/fresh {i:03}.mp3"),
            title: format!("Fresh {i}"),
            artist: Some("Ivo Marsh".into()),
            audio: u64::MAX / 4 + i,
            items: track(u64::MAX / 4 + i, len),
        });
    }
    let new_ids = db.add(&new);
    group(&db);
    compare_fingerprints(&matcher, &db);
    let later = decide(&mut names, &db);
    let after_later = group(&db);

    let idle = decide(&mut names, &db);
    let mut fresh = Names::default();
    let restarted = decide(&mut fresh, &db);
    let restarted_names = fresh.files();
    // One more file, a re-encode of a lone track, decided by a handler
    // that kept nothing.
    let lone = &files[planted.singles[60]];
    db.add(&[Made {
        name: format!("late/{}.mp3", lone.title),
        title: lone.title.clone(),
        artist: lone.artist.clone(),
        audio: u64::MAX / 16,
        items: reencoded(&lone.items, 0xFEED),
    }]);
    group(&db);
    compare_fingerprints(&matcher, &db);
    let restarted_one = decide(&mut Names::default(), &db);
    group(&db);
    let idle_pass = group(&db);
    Report {
        files: files.len(),
        planted,
        ids,
        placing,
        comparing,
        first,
        after_first,
        new_ids,
        later,
        after_later,
        idle,
        restarted,
        restarted_names,
        restarted_one,
        idle_pass,
        db,
    }
}

fn secs(d: Duration) -> String {
    format!("{:8.2} s", d.as_secs_f64())
}

fn print(report: &Report) {
    let db = &report.db;
    println!();
    println!("grouping_scale: {} files", report.files);
    let pass = |name: &str, (s, took): &(Summary, Duration)| {
        println!(
            "  {name:<28}{}   {} placed, {} moved, {} tracks made, {} removed, {} links made; \
             sets held: {} disagreeing, {} both in the Library, {} waiting",
            secs(*took),
            s.placed,
            s.moved,
            s.recordings_made,
            s.recordings_removed,
            s.links_made,
            s.sets_disagreeing,
            s.sets_both_in_library,
            s.sets_waiting
        );
    };
    let run = |name: &str, (d, took): &(Decided, Duration)| {
        println!(
            "  {name:<28}{}   {} files decided, {} pairs asked, {} stored, {} compared directly, \
             {} completed; skipped: {} titles, {} pairs",
            secs(*took),
            d.files,
            d.pairs,
            d.recorded,
            d.compared_directly,
            d.completed,
            d.skipped_buckets,
            d.skipped_pairs
        );
    };
    pass("quick pass (placing)", &report.placing);
    println!(
        "  {:<28}{}   (the matching pass: not this job's)",
        "comparing fingerprints",
        secs(report.comparing)
    );
    run("deciding run, first", &report.first);
    pass("grouping pass after it", &report.after_first);
    run("deciding run, 100 new files", &report.later);
    pass("grouping pass after it", &report.after_later);
    run("deciding run, nothing due", &report.idle);
    run("  the same after a restart", &report.restarted);
    run("one new file after a restart", &report.restarted_one);
    pass("grouping pass, nothing to do", &report.idle_pass);
    println!(
        "  tracks: {}   pairs stored: {}   version links: {}   stored comparisons: {}",
        db.count("SELECT count(*) FROM recording"),
        db.count("SELECT count(*) FROM grouping_pair"),
        db.count("SELECT count(*) FROM version_link"),
        db.count("SELECT count(*) FROM fingerprint_match")
    );
    // Every stored pair, by what the rules said and where it stands.
    let by_state: Vec<(String, String, i64)> = db
        .writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT outcome, state, count(*) FROM grouping_pair GROUP BY 1, 2 ORDER BY 1, 2",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect()
        })
        .unwrap();
    for (outcome, state, pairs) in by_state {
        println!("  pairs: {outcome:<10} {state:<16} {pairs}");
    }
    let held = held_sets(db);
    println!(
        "  held sets: {}   files in them: {}   the largest: {} files",
        held.len(),
        held.iter().sum::<usize>(),
        held.iter().max().copied().unwrap_or(0)
    );
    // Why each held set is held: the pairs between its files that aren't
    // duplicates.
    let causes: Vec<(String, String, i64)> = db
        .writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT p.outcome, p.state, count(*) FROM grouping_pair p
                 WHERE p.outcome <> 'duplicate'
                   AND EXISTS (SELECT 1 FROM grouping_pair h
                               WHERE h.state = 'set_disagrees'
                                 AND (h.file_a IN (p.file_a, p.file_b)
                                      OR h.file_b IN (p.file_a, p.file_b)))
                 GROUP BY 1, 2 ORDER BY 1, 2",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect()
        })
        .unwrap();
    for (outcome, state, pairs) in causes {
        println!("  held because of: {outcome:<10} {state:<16} {pairs} pairs");
    }
    println!("  release build: {}", !cfg!(debug_assertions));
    println!();
}

/// The sets held as disagreeing, as their sizes in files: the files joined
/// by duplicate pairs that stand as `set_disagrees`.
fn held_sets(db: &Db) -> Vec<usize> {
    let pairs: Vec<(i64, i64)> = db
        .writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT file_a, file_b FROM grouping_pair WHERE state = 'set_disagrees'",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap();
    let mut set_of: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    fn find(set_of: &mut std::collections::HashMap<i64, i64>, file: i64) -> i64 {
        let parent = *set_of.entry(file).or_insert(file);
        if parent == file {
            return file;
        }
        let root = find(set_of, parent);
        set_of.insert(file, root);
        root
    }
    for (a, b) in pairs {
        let (a, b) = (find(&mut set_of, a), find(&mut set_of, b));
        if a != b {
            set_of.insert(a.max(b), a.min(b));
        }
    }
    let files: Vec<i64> = set_of.keys().copied().collect();
    let mut sizes: std::collections::BTreeMap<i64, usize> = std::collections::BTreeMap::new();
    for file in files {
        *sizes.entry(find(&mut set_of, file)).or_default() += 1;
    }
    sizes.into_values().collect()
}

/// What must hold whatever the size.
fn check(report: &Report) {
    let db = &report.db;
    let id = |i: usize| report.ids[i];
    let together = |a: usize, b: usize| db.track_of(id(a)) == db.track_of(id(b));
    let all = report.files as u64;
    assert_eq!(report.placing.0.placed, all);
    assert_eq!(report.first.0.files, all, "every file is decided");
    for &(a, b) in &report.planted.duplicates {
        assert!(together(a, b), "duplicates {a} and {b} are two tracks");
    }
    for &(a, b) in &report.planted.unproposed {
        assert!(together(a, b), "the unproposed copy of {a} isn't with it");
    }
    // Only same-named pairs the pass never proposed are compared directly:
    // at most the planted ones (the pass does propose some of them: a
    // flipped bit can make an item equal its neighbour) and the far ends of
    // the sets that disagree. Never anything like every pair.
    let directly = report.first.0.compared_directly as usize;
    let most = report.planted.unproposed.len() + report.planted.disagreeing.len();
    assert!(
        (1..=most).contains(&directly),
        "{directly} pairs compared directly, expected 1 to {most}"
    );
    for &(a, b) in &report.planted.cuts {
        assert!(!together(a, b), "the cut {b} merged into {a}");
        assert!(db.linked(id(a), id(b)), "the cut {b} isn't linked to {a}");
    }
    for &[first, second, third] in &report.planted.disagreeing {
        assert!(
            !together(first, second) && !together(second, third) && !together(first, third),
            "files {first}, {second} and {third} disagree, and some of them merged"
        );
    }
    assert_eq!(
        report.after_first.0.sets_disagreeing as usize,
        report.planted.disagreeing.len()
    );
    // "Intro": one title too many files share, skipped and counted once.
    let intros = INTROS.min(report.files / 4) as u64;
    assert_eq!(report.first.0.skipped_buckets, 1);
    assert_eq!(report.first.0.skipped_pairs, intros * (intros - 1) / 2);
    // 100 new files: only they are decided, and each re-encode joins.
    assert_eq!(report.later.0.files, 100);
    assert_eq!(report.after_later.0.moved, 50);
    // The first 50 of them are the re-encodes, of the first 50 lone tracks.
    for (&new, &single) in report.new_ids.iter().zip(&report.planted.singles).take(50) {
        assert_eq!(db.track_of(new), db.track_of(id(single)));
    }
    // Nothing due: nothing asked, nothing written, with or without the
    // index kept.
    assert_eq!(report.idle.0, Decided::default());
    assert_eq!(report.restarted.0, Decided::default());
    assert_eq!(report.restarted_names, 0, "no names read with nothing due");
    assert_eq!(report.restarted_one.0.files, 1);
    // Every held set is one of the planted ones: three files.
    let held = held_sets(db);
    assert_eq!(held.len(), report.planted.disagreeing.len());
    assert!(held.iter().all(|&files| files == 3), "{held:?}");
    assert!(!report.idle_pass.0.changed_tracks());
}

#[test]
fn a_small_library_goes_through_the_same_steps() {
    let report = steps(2_000, 400);
    print(&report);
    check(&report);
    // The pairs asked about grow with the files, not with their square:
    // 2,000 files have two million pairs.
    assert!(
        report.first.0.pairs < 4 * report.files as u64,
        "{} pairs asked about for {} files",
        report.first.0.pairs,
        report.files
    );
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn a_library_of_100k() {
    let report = steps(100_000, 600);
    print(&report);
    check(&report);
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn a_library_of_10k() {
    let report = steps(10_000, 600);
    print(&report);
    check(&report);
}
