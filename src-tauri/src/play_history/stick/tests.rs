//! Gig-stick history (3.6): decoding synthetic `export.pdb` files, reading a
//! stick through staging without touching it, and the import with its Undo.
//! Made-up names and rows only; no real stick's data.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;

use super::devicesql::{parse, Export, HistoryList, StickPlay};
use super::fixture::{self, Pdb, ARTISTS, ENTRIES, LISTS, PAGE, TRACKS};
use super::*;
use crate::play_history::{plays, sessions, set_date, HistoryOutcome};
use crate::relink::Mounted;
use crate::search::tests::support::{Lib, Mount};

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
        .table(TRACKS, vec![fixture::track(1, 5, &long_title, "/Contents/x.mp3")], 16)
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
    assert_eq!(read_stick(&s.guard, &s.stick), Err(StickRefusal::NotAnExport));
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
    let rb = s.stick.join("PIONEER").join("rekordbox");
    fs::write(rb.join("exportLibrary.db"), b"made-up OneLibrary bytes").unwrap();
    let music = s.stick.join("Contents").join("Made Up Artist");
    fs::create_dir_all(&music).unwrap();
    fs::write(music.join("first.mp3"), b"not audio").unwrap();
    // Read-only files: a read that asked to write would fail.
    for file in [export_path(&s.stick), rb.join("exportLibrary.db")] {
        set_readonly(&file, true);
    }

    let mut before = BTreeMap::new();
    snapshot(&s.stick, &mut before);
    assert!(before.len() >= 6, "{before:?}");
    read_stick(&s.guard, &s.stick).unwrap();
    let mut after = BTreeMap::new();
    snapshot(&s.stick, &mut after);
    assert_eq!(before, after, "the stick changed");

    for file in [export_path(&s.stick), rb.join("exportLibrary.db")] {
        set_readonly(&file, false);
    }
}

#[test]
fn a_listing_names_each_list_with_its_number_of_plays() {
    let listed = listing(&two_gigs());
    let StickListing::Listed { lists, skipped } = listed else {
        panic!("{listed:?}");
    };
    assert_eq!(skipped, 0);
    let got: Vec<_> = lists.iter().map(|l| (l.id, l.name.as_str(), l.plays)).collect();
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

fn import(lib: &Lib, root: &'static str, export: Export, chosen: Vec<u32>, day: &'static str) -> StickImport {
    run(lib, move |c| {
        let m = Mounted::ask(&crate::relink::identities(c).unwrap(), &Mount);
        import_stick(c, &m, root, &export, &chosen, day)
    })
}

fn operations(lib: &Lib) -> i64 {
    read(lib, |c| c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0)))
}

fn library() -> (Lib, i64) {
    let lib = Lib::new();
    let (a, _) = lib.track(Some("Track A"), Some("Artist A"), "a.mp3");
    (lib, a)
}

#[test]
fn importing_lists_makes_one_session_each_in_entry_order_on_the_chosen_date() {
    let (lib, a) = library();
    let outcome = import(&lib, r"E:\", two_gigs(), vec![1, 2], "2026-04-18");
    let StickImport::Done {
        operation_id,
        sessions: made,
        plays: total,
        matched,
        already_imported,
        skipped,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert!(operation_id.is_some());
    assert_eq!((made.len(), total, matched, already_imported, skipped), (2, 5, 1, 0, 0));
    assert_eq!(operations(&lib), 1, "one import is one operation");

    let list = read(&lib, sessions);
    let mut got: Vec<_> = list
        .iter()
        .map(|s| (s.source.as_str(), s.name.as_str(), s.played_on.as_str(), s.plays, s.matched))
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
        .map(|p| (p.position, p.recording_id, p.artist.as_deref(), p.path.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (0, None, Some("Made Up Artist"), r"E:\Contents\Made Up Artist\first.mp3"),
            (1, None, Some("Другой Артист"), r"E:\Contents\Другой\second.flac"),
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
            already_imported: 2,
            skipped: 0,
        }
    );
    assert_eq!(operations(&lib), 2, "nothing new is no operation");
    // The same set on another night is another gig.
    let other_night = import(&lib, r"E:\", two_gigs(), vec![1], "2026-04-25");
    assert!(matches!(other_night, StickImport::Done { already_imported: 0, .. }));
}

#[test]
fn undoing_a_stick_import_takes_all_its_sessions_and_plays_away() {
    let (lib, _) = library();
    import(&lib, r"E:\", two_gigs(), vec![1, 2], "2026-04-18");
    let undone = run(&lib, |c| ops::undo_for_user(c, None));
    assert!(matches!(undone, ops::UndoOutcome::Undone { .. }), "{undone:?}");
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
fn a_path_that_climbs_out_of_the_stick_matches_nothing_but_is_kept() {
    let (lib, _) = library();
    let export = Export {
        lists: vec![HistoryList {
            id: 1,
            name: "HISTORY 001".into(),
            plays: vec![play("/../Music/a.mp3", Some("Made Up Artist"), "Up")],
        }],
        skipped: 0,
    };
    let outcome = import(&lib, r"E:\Stick", export, vec![1], "2026-04-18");
    assert!(
        matches!(outcome, StickImport::Done { plays: 1, matched: 0, .. }),
        "{outcome:?}"
    );
}

#[test]
fn a_stick_read_end_to_end_imports_its_plays_unmatched_with_their_names() {
    let s = sandbox();
    fixture::write_stick(&s.stick, &fixture::two_gigs().build());
    let export = read_stick(&s.guard, &s.stick).unwrap();
    let (lib, _) = library();
    let root: &'static str = Box::leak(s.stick.display().to_string().into_boxed_str());
    let outcome = import(&lib, root, export, vec![1], "2026-04-18");
    assert!(
        matches!(outcome, StickImport::Done { plays: 2, matched: 0, .. }),
        "{outcome:?}"
    );
}
