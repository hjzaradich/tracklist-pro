//! Gig-stick history (3.6): decoding synthetic `export.pdb` files, reading a
//! stick through staging without touching it, and the import with its Undo.
//! Made-up names and rows only; no real stick's data.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;

use super::devicesql::{parse, Export, HistoryList, StickPlay};
use super::fixture::{self, Pdb, ARTISTS, ENTRIES, LISTS, PAGE, TRACKS};
use super::*;
use crate::play_history::{plays, sessions, set_date, HistoryOutcome};
use crate::search::tests::support::Lib;
use crate::tags::test_audio;

fn play(path: &str, artist: Option<&str>, title: &str) -> StickPlay {
    StickPlay {
        path: path.to_owned(),
        artist: artist.map(str::to_owned),
        title: Some(title.to_owned()),
    }
}

fn two_gigs() -> Export {
    parse(&fixture::two_gigs().build()).unwrap()
}

fn lists_of(export: &Export) -> Vec<(u32, &str, usize)> {
    export
        .lists
        .iter()
        .map(|l| (l.id, l.name.as_str(), l.plays.len()))
        .collect()
}

/// Sets the `u32` or `u16` at `at` in `bytes`.
fn put32(bytes: &mut [u8], at: usize, v: u32) {
    bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put16(bytes: &mut [u8], at: usize, v: u16) {
    bytes[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn page_start(pdb: &Pdb, kind: u32, n: usize) -> usize {
    pdb.pages_of(kind)[n] as usize * PAGE
}

// ---- decoding -------------------------------------------------------------------

#[test]
fn a_sticks_history_lists_are_read_with_their_plays_in_entry_order() {
    let export = two_gigs();
    assert_eq!(export.skipped, 0);
    assert_eq!(
        lists_of(&export),
        vec![(1, "HISTORY 001", 2), (2, "HISTORY 002", 3)]
    );
    let first = play(
        "/Contents/Made Up Artist/first.mp3",
        Some("Made Up Artist"),
        "First Tune",
    );
    let second = play(
        "/Contents/Другой/second.flac",
        Some("Другой Артист"),
        "Second Tune",
    );
    assert_eq!(export.lists[0].plays, vec![first.clone(), second.clone()]);
    // Stored out of order; read by the place each entry gives. The third
    // track's artist isn't on the stick.
    assert_eq!(
        export.lists[1].plays,
        vec![first, second, play("/Music/a.mp3", None, "Third Tune")]
    );
}

#[test]
fn long_strings_and_far_artist_names_are_read() {
    let long_title = "A Very Long Made Up Title ".repeat(8);
    let mut far = vec![0x64, 0, 0, 0];
    far.extend(5_u32.to_le_bytes());
    far.extend([0x03, 0x00]);
    far.extend(0x0c_u16.to_le_bytes());
    far.extend(fixture::dstring("Far Name"));
    let bytes = Pdb::new()
        .table(ARTISTS, vec![far], 16)
        .table(
            TRACKS,
            vec![fixture::track(1, 5, &long_title, "/Contents/x.mp3")],
            16,
        )
        .table(LISTS, vec![fixture::list(7, "HISTORY 007")], 16)
        .table(ENTRIES, vec![fixture::entry(1, 7, 1)], 16)
        .build();
    let export = parse(&bytes).unwrap();
    assert_eq!(export.skipped, 0);
    assert_eq!(
        export.lists[0].plays,
        vec![play("/Contents/x.mp3", Some("Far Name"), long_title.trim())]
    );
}

#[test]
fn what_isnt_an_export_database_is_refused_without_a_panic() {
    assert_eq!(parse(b""), None);
    assert_eq!(parse(b"#EXTM3U\nnot a database"), None);
    let mut bytes = fixture::two_gigs().build();
    put32(&mut bytes, 4, 0);
    assert_eq!(parse(&bytes), None, "a page length of 0");
    put32(&mut bytes, 4, u32::MAX);
    assert_eq!(parse(&bytes), None, "a page length past the file");
}

#[test]
fn a_truncated_file_keeps_what_it_holds_and_counts_the_rest() {
    let pdb = fixture::two_gigs();
    let bytes = pdb.build();
    // Cut inside the entries' page: the lists are there, their plays aren't.
    let cut = &bytes[..page_start(&pdb, ENTRIES, 0) + 100];
    let export = parse(cut).unwrap();
    assert_eq!(
        lists_of(&export),
        vec![(1, "HISTORY 001", 0), (2, "HISTORY 002", 0)]
    );
    assert!(export.skipped >= 1, "{export:?}");
    // Cut anywhere: never a panic.
    for len in (0..bytes.len()).step_by(61) {
        let _ = parse(&bytes[..len]);
    }
}

#[test]
fn a_bad_page_is_skipped_and_counted_and_the_rest_is_read() {
    let pdb = fixture::two_gigs();
    let mut bytes = pdb.build();
    // The tracks' second page (the third track) says it's another table's.
    put32(&mut bytes, page_start(&pdb, TRACKS, 1) + 8, 99);
    let export = parse(&bytes).unwrap();
    // The page, and the entry of the track it held.
    assert_eq!(export.skipped, 2);
    assert_eq!(
        lists_of(&export),
        vec![(1, "HISTORY 001", 2), (2, "HISTORY 002", 2)]
    );
}

#[test]
fn out_of_range_offsets_and_huge_counts_are_skipped_and_counted() {
    let pdb = fixture::two_gigs();
    let clean = pdb.build();

    // A row's offset past the page's end.
    let mut bytes = clean.clone();
    let entries = page_start(&pdb, ENTRIES, 0);
    put16(&mut bytes, entries + PAGE - 6, 0xffff);
    let export = parse(&bytes).unwrap();
    assert_eq!(export.skipped, 1);
    assert_eq!(export.lists[0].plays.len(), 1);

    // A string offset past the page's end: the track and its entries go.
    let mut bytes = clean.clone();
    let tracks = page_start(&pdb, TRACKS, 0);
    put16(&mut bytes, tracks + 0x28 + 0x5e + 2 * 20, 0xfff0);
    let export = parse(&bytes).unwrap();
    assert_eq!(export.skipped, 3, "the row and its two entries");

    // A huge row count: the groups past the page's room are skipped.
    let mut bytes = clean.clone();
    put16(&mut bytes, entries + 0x22, 0x1ffe);
    let export = parse(&bytes).unwrap();
    assert!(export.skipped >= 1, "{export:?}");

    // A huge table count, a first page far past the end, a chain that loops.
    let mut bytes = clean.clone();
    put32(&mut bytes, 8, u32::MAX);
    put32(&mut bytes, 0x1c + 8, 0x7fff_ffff);
    let lists = page_start(&pdb, LISTS, 0);
    put32(&mut bytes, lists + 12, pdb.pages_of(LISTS)[0]);
    put32(&mut bytes, 0x1c + 2 * 16 + 12, 0);
    let export = parse(&bytes).unwrap();
    assert!(export.skipped >= 3, "{export:?}");
    assert_eq!(export.lists.len(), 2, "the lists are still read");
}

#[test]
fn no_single_corrupt_byte_makes_the_reader_panic() {
    let bytes = fixture::two_gigs().build();
    for at in (0..bytes.len()).step_by(7) {
        for value in [0x00, 0xff, 0x7f] {
            let mut bad = bytes.clone();
            bad[at] = value;
            let _ = parse(&bad);
        }
    }
}

// ---- reading a stick -------------------------------------------------------------

struct Sandbox {
    _dir: tempfile::TempDir,
    data: PathBuf,
    stick: PathBuf,
    guard: WriteGuard,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let data = base.join("data");
    fs::create_dir_all(&data).unwrap();
    let stick = base.join("stick");
    fs::create_dir_all(&stick).unwrap();
    let guard = WriteGuard::app_data(&data).unwrap();
    Sandbox {
        _dir: dir,
        data,
        stick,
        guard,
    }
}

impl Sandbox {
    fn staging(&self) -> PathBuf {
        self.data
            .join(crate::write_guard::STAGING_DIR)
            .join(STAGING)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    bytes: Option<Vec<u8>>,
    modified: SystemTime,
    readonly: bool,
}

/// Every entry under `dir` with its bytes, modified time and read-only flag.
fn snapshot(dir: &Path, out: &mut BTreeMap<PathBuf, Seen>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        out.insert(
            path.clone(),
            Seen {
                bytes: meta.is_file().then(|| fs::read(&path).unwrap()),
                modified: meta.modified().unwrap(),
                readonly: meta.permissions().readonly(),
            },
        );
        if meta.is_dir() {
            snapshot(&path, out);
        }
    }
}

fn set_readonly(path: &Path, on: bool) {
    let mut perms = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(on);
    fs::set_permissions(path, perms).unwrap();
}

#[test]
fn reading_a_stick_reads_a_copy_in_staging_and_removes_it() {
    let s = sandbox();
    fixture::write_stick(&s.stick, &fixture::two_gigs().build());
    let export = read_stick(&s.guard, &s.stick).unwrap();
    assert_eq!(export, two_gigs());
    assert!(!s.staging().exists(), "staging is removed after a read");
}

#[test]
fn a_failed_read_removes_staging_too_and_says_why() {
    let s = sandbox();
    assert_eq!(read_stick(&s.guard, &s.stick), Err(StickRefusal::NoExport));
    fixture::write_stick(&s.stick, b"not a database at all");
    assert_eq!(
        read_stick(&s.guard, &s.stick),
        Err(StickRefusal::NotAnExport)
    );
    assert!(!s.staging().exists(), "staging is removed after a failure");
    // A folder where the file should be.
    let other = s.stick.join("other");
    fs::create_dir_all(export_path(&other)).unwrap();
    assert!(read_stick(&s.guard, &other).is_err());
    assert!(!s.staging().exists());
}

#[test]
fn reading_a_stick_never_changes_anything_on_it() {
    let s = sandbox();
    fixture::write_stick(&s.stick, &fixture::two_gigs().build());
    // OneLibrary's database sits beside it and is left alone; music too.
    // ci-reads: not a frontend read
    let rb = s.stick.join("PIONEER").join("rekordbox");
    fs::write(rb.join("exportLibrary.db"), b"made-up OneLibrary bytes").unwrap();
    let music = s.stick.join("Contents").join("Made Up Artist");
    fs::create_dir_all(&music).unwrap();
    fs::write(music.join("first.mp3"), b"not audio").unwrap();
    // Read-only files: a read that asked to write would fail.
    let files = [
        export_path(&s.stick),
        rb.join("exportLibrary.db"),
        music.join("first.mp3"),
    ];
    for file in &files {
        set_readonly(file, true);
    }

    let mut before = BTreeMap::new();
    snapshot(&s.stick, &mut before);
    assert!(before.len() >= 6, "{before:?}");
    // Reading its database, then hashing the files its lists play.
    let export = read_stick(&s.guard, &s.stick).unwrap();
    let contents = hash_stick_files(&s.stick.display().to_string(), &export, &[1, 2]);
    assert!(matches!(
        contents.get("/Contents/Made Up Artist/first.mp3"),
        Some(Some(_))
    ));
    let mut after = BTreeMap::new();
    snapshot(&s.stick, &mut after);
    assert_eq!(before, after, "the stick changed");

    for file in &files {
        set_readonly(file, false);
    }
}

#[test]
fn a_listing_names_each_list_with_its_number_of_plays() {
    let listed = listing(&two_gigs());
    let StickListing::Listed { lists, skipped } = listed else {
        panic!("{listed:?}");
    };
    assert_eq!(skipped, 0);
    let got: Vec<_> = lists
        .iter()
        .map(|l| (l.id, l.name.as_str(), l.plays))
        .collect();
    assert_eq!(got, vec![(1, "HISTORY 001", 2), (2, "HISTORY 002", 3)]);
}

// ---- importing ----------------------------------------------------------------

fn run<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&mut Connection) -> Result<T, OpsError> + Send + 'static,
) -> T {
    lib.writer.call(move |c| Ok(job(c))).unwrap().unwrap()
}

fn read<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    lib.writer.call(move |c| job(c)).unwrap()
}

fn import_with(
    lib: &Lib,
    root: &str,
    export: Export,
    chosen: Vec<u32>,
    day: &'static str,
    contents: HashMap<String, Option<Content>>,
) -> StickImport {
    let root = root.to_owned();
    run(lib, move |c| {
        import_stick(c, &root, &export, &chosen, day, &contents)
    })
}

/// An import whose stick files couldn't be read.
fn import(
    lib: &Lib,
    root: &str,
    export: Export,
    chosen: Vec<u32>,
    day: &'static str,
) -> StickImport {
    import_with(lib, root, export, chosen, day, HashMap::new())
}

/// Gives `file` the hashes of `content`, as the hash stage would.
fn set_hashes(lib: &Lib, file: i64, content: Content) {
    read(lib, move |c| {
        c.execute(
            "UPDATE file SET audio_hash = ?1, blake3 = ?2 WHERE id = ?3",
            rusqlite::params![
                content.audio.map(|a| a.to_vec()),
                content.whole.to_vec(),
                file
            ],
        )
    });
}

const AUDIO_A: Content = Content {
    audio: Some([7; AUDIO_HASH_LEN]),
    whole: [8; 32],
};

fn operations(lib: &Lib) -> i64 {
    read(lib, |c| {
        c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0))
    })
}

fn library() -> (Lib, i64) {
    let lib = Lib::new();
    let (a, file) = lib.track(Some("Track A"), Some("Artist A"), "a.mp3");
    set_hashes(&lib, file, AUDIO_A);
    (lib, a)
}

#[test]
fn importing_lists_makes_one_session_each_in_entry_order_on_the_chosen_date() {
    let (lib, a) = library();
    // The third track's file is track A's audio; the others weren't read.
    let contents = HashMap::from([("/Music/a.mp3".to_owned(), Some(AUDIO_A))]);
    let outcome = import_with(&lib, r"E:\", two_gigs(), vec![1, 2], "2026-04-18", contents);
    let StickImport::Done {
        operation_id,
        sessions: made,
        plays: total,
        matched,
        ambiguous,
        unreadable,
        already_imported,
        skipped,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert!(operation_id.is_some());
    assert_eq!(
        (
            made.len(),
            total,
            matched,
            ambiguous,
            unreadable,
            already_imported,
            skipped
        ),
        (2, 5, 1, 0, 4, 0, 0)
    );
    assert_eq!(operations(&lib), 1, "one import is one operation");

    let list = read(&lib, sessions);
    let mut got: Vec<_> = list
        .iter()
        .map(|s| {
            (
                s.source.as_str(),
                s.name.as_str(),
                s.played_on.as_str(),
                s.plays,
                s.matched,
            )
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("stick", "HISTORY 001", "2026-04-18", 2, 0),
            ("stick", "HISTORY 002", "2026-04-18", 3, 1),
        ]
    );

    let second = list.iter().find(|s| s.name == "HISTORY 002").unwrap().id;
    let reply = read(&lib, move |c| plays(c, second));
    let rows: Vec<_> = reply
        .plays
        .iter()
        .map(|p| {
            (
                p.position,
                p.recording_id,
                p.artist.as_deref(),
                p.path.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                0,
                None,
                Some("Made Up Artist"),
                r"E:\Contents\Made Up Artist\first.mp3"
            ),
            (
                1,
                None,
                Some("Другой Артист"),
                r"E:\Contents\Другой\second.flac"
            ),
            (2, Some(a), None, r"E:\Music\a.mp3"),
        ]
    );

    // The date can be corrected like any session's.
    let fixed = run(&lib, move |c| set_date(c, second, "2026-04-19"));
    assert!(matches!(fixed, HistoryOutcome::Done { .. }), "{fixed:?}");
}

#[test]
fn only_the_chosen_lists_are_imported() {
    let (lib, _) = library();
    let outcome = import(&lib, r"E:\", two_gigs(), vec![2], "2026-04-18");
    assert!(matches!(outcome, StickImport::Done { ref sessions, .. } if sessions.len() == 1));
    let names: Vec<String> = read(&lib, sessions).into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["HISTORY 002"]);
}

#[test]
fn the_same_list_on_the_same_day_is_left_out_as_already_imported() {
    let (lib, _) = library();
    import(&lib, r"E:\", two_gigs(), vec![1], "2026-04-18");
    let again = import(&lib, r"E:\", two_gigs(), vec![1, 2], "2026-04-18");
    assert!(
        matches!(again, StickImport::Done { already_imported: 1, ref sessions, .. } if sessions.len() == 1),
        "{again:?}"
    );
    let once_more = import(&lib, r"E:\", two_gigs(), vec![1, 2], "2026-04-18");
    assert_eq!(
        once_more,
        StickImport::Done {
            operation_id: None,
            sessions: Vec::new(),
            plays: 0,
            matched: 0,
            ambiguous: 0,
            unreadable: 0,
            already_imported: 2,
            skipped: 0,
        }
    );
    assert_eq!(operations(&lib), 2, "nothing new is no operation");
    // The same set on another night is another gig.
    let other_night = import(&lib, r"E:\", two_gigs(), vec![1], "2026-04-25");
    assert!(matches!(
        other_night,
        StickImport::Done {
            already_imported: 0,
            ..
        }
    ));
}

#[test]
fn undoing_a_stick_import_takes_all_its_sessions_and_plays_away() {
    let (lib, _) = library();
    import(&lib, r"E:\", two_gigs(), vec![1, 2], "2026-04-18");
    let undone = run(&lib, |c| ops::undo_for_user(c, None));
    assert!(
        matches!(undone, ops::UndoOutcome::Undone { .. }),
        "{undone:?}"
    );
    let left: (i64, i64) = read(&lib, |c| {
        c.query_row(
            "SELECT (SELECT count(*) FROM session), (SELECT count(*) FROM play_event)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    });
    assert_eq!(left, (0, 0));
}

#[test]
fn a_bad_date_or_no_list_with_plays_is_refused() {
    let (lib, _) = library();
    let refused = |reason| StickImport::Refused { reason };
    assert_eq!(
        import(&lib, r"E:\", two_gigs(), vec![1], "2026-02-30"),
        refused(StickRefusal::BadDate)
    );
    assert_eq!(
        import(&lib, r"E:\", two_gigs(), vec![99], "2026-04-18"),
        refused(StickRefusal::NothingChosen)
    );
    let empty = Export {
        lists: vec![HistoryList {
            id: 3,
            name: "HISTORY 003".into(),
            plays: Vec::new(),
        }],
        skipped: 0,
    };
    assert_eq!(
        import(&lib, r"E:\", empty, vec![3], "2026-04-18"),
        refused(StickRefusal::NothingChosen)
    );
    assert_eq!(operations(&lib), 0);
}

#[test]
fn a_path_that_climbs_out_of_the_stick_is_never_opened_and_its_play_is_kept() {
    let s = sandbox();
    // A real file beside the stick, which the path would reach.
    let beside = s.stick.parent().unwrap().join("outside.wav");
    fs::write(&beside, test_audio::wav()).unwrap();
    let export = Export {
        lists: vec![HistoryList {
            id: 1,
            name: "HISTORY 001".into(),
            plays: vec![play("/../outside.wav", Some("Made Up Artist"), "Up")],
        }],
        skipped: 0,
    };
    let root = s.stick.display().to_string();
    let contents = hash_stick_files(&root, &export, &[1]);
    assert!(contents.is_empty(), "{contents:?}");
    let (lib, _) = library();
    let outcome = import_with(&lib, &root, export, vec![1], "2026-04-18", contents);
    assert!(
        matches!(
            outcome,
            StickImport::Done {
                plays: 1,
                matched: 0,
                unreadable: 1,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn a_stick_read_end_to_end_imports_its_plays_unmatched_with_their_names() {
    let s = sandbox();
    fixture::write_stick(&s.stick, &fixture::two_gigs().build());
    let export = read_stick(&s.guard, &s.stick).unwrap();
    let (lib, _) = library();
    let outcome = import(
        &lib,
        &s.stick.display().to_string(),
        export,
        vec![1],
        "2026-04-18",
    );
    assert!(
        matches!(
            outcome,
            StickImport::Done {
                plays: 2,
                matched: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
}

// ---- matching by content ------------------------------------------------------

const STICK_FILE: &str = "/Contents/Made Up Artist/one.wav";

/// A stick whose one list plays one file, `bytes` (none when `None`).
fn stick_playing(s: &Sandbox, bytes: Option<&[u8]>) -> Export {
    let pdb = Pdb::new()
        .table(
            TRACKS,
            vec![fixture::track(1, 0, "One Tune", STICK_FILE)],
            16,
        )
        .table(LISTS, vec![fixture::list(1, "HISTORY 001")], 16)
        .table(ENTRIES, vec![fixture::entry(1, 1, 1)], 16);
    fixture::write_stick(&s.stick, &pdb.build());
    if let Some(bytes) = bytes {
        let dir = s.stick.join("Contents").join("Made Up Artist");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("one.wav"), bytes).unwrap();
    }
    read_stick(&s.guard, &s.stick).unwrap()
}

/// What the hash stage would store for `bytes`.
fn content_of(bytes: &[u8]) -> Content {
    let mut buf = vec![0_u8; 4096];
    let h = crate::hash::hash_reader(&mut std::io::Cursor::new(bytes), &mut buf, &mut |_| false)
        .unwrap()
        .unwrap();
    Content {
        audio: h.audio.ok().map(|a| a.to_bytes()),
        whole: h.blake3,
    }
}

/// `wav` with a title tag added: other bytes, the same audio.
fn retagged(wav: &[u8]) -> Vec<u8> {
    let mut out = wav.to_vec();
    let body = b"INFOINAM\x06\x00\x00\x00Other\x00";
    out.extend(b"LIST");
    out.extend(u32::try_from(body.len()).unwrap().to_le_bytes());
    out.extend(body);
    let riff = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

/// A library with one track whose file holds `bytes`.
fn library_of(bytes: &[u8]) -> (Lib, i64) {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Track A"), Some("Artist A"), "a.wav");
    set_hashes(&lib, file, content_of(bytes));
    (lib, track)
}

/// Hashes the stick's files and imports its list; the play's track.
fn import_matched(s: &Sandbox, lib: &Lib, export: Export) -> (StickImport, Option<i64>) {
    let root = s.stick.display().to_string();
    let contents = hash_stick_files(&root, &export, &[1]);
    let outcome = import_with(lib, &root, export, vec![1], "2026-04-18", contents);
    let track = read(lib, |c| {
        c.query_row("SELECT recording_id FROM play_event", [], |r| r.get(0))
    });
    (outcome, track)
}

#[test]
fn a_byte_copy_on_the_stick_matches_its_track() {
    let s = sandbox();
    let wav = test_audio::wav();
    let (lib, track) = library_of(&wav);
    let export = stick_playing(&s, Some(&wav));
    let (outcome, found) = import_matched(&s, &lib, export);
    assert_eq!(found, Some(track), "{outcome:?}");
    assert!(matches!(outcome, StickImport::Done { matched: 1, .. }));
}

#[test]
fn a_retagged_copy_matches_by_its_audio() {
    let s = sandbox();
    let wav = test_audio::wav();
    let copy = retagged(&wav);
    assert_ne!(content_of(&copy).whole, content_of(&wav).whole);
    assert_eq!(content_of(&copy).audio, content_of(&wav).audio);
    assert!(content_of(&wav).audio.is_some());
    let (lib, track) = library_of(&wav);
    let (outcome, found) = import_matched(&s, &lib, stick_playing(&s, Some(&copy)));
    assert_eq!(found, Some(track), "{outcome:?}");
}

#[test]
fn a_file_with_other_audio_matches_nothing_and_keeps_its_names() {
    let s = sandbox();
    let wav = test_audio::wav();
    let mut other = wav.clone();
    let last = other.len() - 3;
    other[last] ^= 0x55;
    let (lib, _) = library_of(&wav);
    let (outcome, found) = import_matched(&s, &lib, stick_playing(&s, Some(&other)));
    assert_eq!(found, None);
    assert!(
        matches!(
            outcome,
            StickImport::Done {
                plays: 1,
                matched: 0,
                ambiguous: 0,
                unreadable: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
    let title: Option<String> = read(&lib, |c| {
        c.query_row("SELECT raw_title FROM play_event", [], |r| r.get(0))
    });
    assert_eq!(title.as_deref(), Some("One Tune"));
}

#[test]
fn audio_that_is_several_tracks_matches_none_of_them() {
    let s = sandbox();
    let wav = test_audio::wav();
    let (lib, _) = library_of(&wav);
    let (_, second) = lib.track(Some("Track B"), Some("Artist B"), "b.wav");
    set_hashes(&lib, second, content_of(&wav));
    let (outcome, found) = import_matched(&s, &lib, stick_playing(&s, Some(&wav)));
    assert_eq!(found, None);
    assert!(
        matches!(
            outcome,
            StickImport::Done {
                matched: 0,
                ambiguous: 1,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn a_file_missing_from_the_stick_leaves_its_play_unmatched_and_counted() {
    let s = sandbox();
    let (lib, _) = library_of(&test_audio::wav());
    let export = stick_playing(&s, None);
    let contents = hash_stick_files(&s.stick.display().to_string(), &export, &[1]);
    assert_eq!(contents.get(STICK_FILE), Some(&None));
    let (outcome, found) = import_matched(&s, &lib, export);
    assert_eq!(found, None);
    assert!(
        matches!(
            outcome,
            StickImport::Done {
                matched: 0,
                unreadable: 1,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn only_the_files_of_the_chosen_lists_are_read() {
    let s = sandbox();
    fixture::write_stick(&s.stick, &fixture::two_gigs().build());
    let export = read_stick(&s.guard, &s.stick).unwrap();
    let contents = hash_stick_files(&s.stick.display().to_string(), &export, &[1]);
    let mut read_paths: Vec<_> = contents.keys().cloned().collect();
    read_paths.sort();
    assert_eq!(
        read_paths,
        [
            "/Contents/Made Up Artist/first.mp3",
            "/Contents/Другой/second.flac"
        ]
    );
}
