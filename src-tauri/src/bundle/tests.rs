use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;

use super::check::Caps;
use super::swap::{
    finish, on_startup, put_back_with, start, start_with, Layout, Swapped, KEEP_BACKUPS,
};
use super::*;
use crate::db::migrations::{self, MIGRATIONS};
use crate::db::{db_path, Writer};
use crate::diagnostics::{EventKind, EventLog, EventReason};
use crate::write_guard::no_writes_elsewhere::snapshot;

/// A small Library: user rows in many tables, AUTOINCREMENT counters, and
/// everything [`MACHINE_ONLY`] takes out.
const SEED: &str = r#"
    INSERT INTO volume (id, identity, label, kind, last_mount_path, guid)
        VALUES (1, 'serial=FS-1234ABCD', 'Music', 'external', 'E:/', '{0d1e}');
    INSERT INTO music_folder (id, volume_id, rel_path, rel_path_key, watch) VALUES (1, 1, 'DJ', 'dj', 1);
    INSERT INTO file (id, music_folder_id, rel_path, rel_path_key) VALUES (7, 1, 'House/a.mp3', 'house/a.mp3');
    INSERT INTO file (id, music_folder_id, rel_path, rel_path_key) VALUES (8, 1, 'House/b.mp3', 'house/b.mp3');
    INSERT INTO embedding (file_id, audio_hash, model_id, fingerprint, label_scores)
        VALUES (7, zeroblob(34), 'ced-tiny@a455633f/labels1', zeroblob(768), zeroblob(228));
    INSERT INTO embedding_failure (file_id, audio_hash, model_id, reason)
        VALUES (8, zeroblob(34), 'ced-tiny@a455633f/labels1', 'damaged');
    INSERT INTO recording (id) VALUES (3);
    INSERT INTO library_track (id, recording_id, linked_file_id) VALUES (5, 3, 7);
    INSERT INTO crate (id, kind, name) VALUES (2, 'static', 'Warmup');
    INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 5);
    INSERT INTO tag_group (id, name) VALUES (1, 'Mood');
    INSERT INTO tag (id, group_id, name) VALUES (1, 1, 'Dark');
    INSERT INTO playlist (id, name, name_key) VALUES (1, 'Friday', 'friday');
    INSERT INTO sync_base (library_track_id, field, value) VALUES (5, 'title', '"A"');
    DELETE FROM sync_base;
    INSERT INTO sync_base (library_track_id, field, value) VALUES (5, 'title', '"A"');
    INSERT INTO operation (id, kind) VALUES (1, 'create_crate');
    INSERT INTO change (operation_id, entity, entity_id, action, field) VALUES (1, 'crate', 2, 'insert', 'name');
    INSERT INTO setting (key, value) VALUES ('key_notation', '"camelot"'), ('energy_as_stars', 'true'),
        ('rekordbox_xml_path', '"C:/Users/dj/rekordbox-secret-path.xml"'), ('rekordbox_xml_watch', 'true'),
        ('rekordbox_xml_last_failure', '{"path":"C:/x.xml"}'), ('read_online_only_files', 'true'),
        ('inbox.incompleteWalks.1', '2'),
        ('update_check_asked', 'true'),
        ('update_check_result', '{"checkedAt":"2026-11-01T00:00:00.000Z","version":"9.0.0","url":"u"}'),
        ('rekordbox_xml_last_read', '{"path":"C:/Users/dj/rekordbox-secret-path.xml","readAt":"2026-11-01T00:00:00Z"}');
    INSERT INTO job (kind) VALUES ('scan');
    INSERT INTO service_optin (service, enabled, enabled_at) VALUES ('musicbrainz', 1, '2026-11-01');
"#;

const MACHINE_SETTINGS: &[&str] = &[
    "rekordbox_xml_path",
    "rekordbox_xml_watch",
    "rekordbox_xml_last_failure",
    "read_online_only_files",
    "inbox.incompleteWalks.1",
];

/// An app data folder at `root/name` with a Library made from `seed`. The
/// writer is still open.
fn library(root: &Path, name: &str, seed: &'static str) -> (WriteGuard, Writer) {
    let guard = WriteGuard::app_data(&root.join(name)).unwrap();
    let writer = Writer::open(&guard.check(&db_path(guard.app_data_dir())).unwrap()).unwrap();
    writer.call(move |c| c.execute_batch(seed)).unwrap();
    (guard, writer)
}

/// The same, closed: ready to be swapped.
fn closed_library(root: &Path, name: &str, seed: &'static str) -> WriteGuard {
    let (guard, writer) = library(root, name, seed);
    writer.close();
    guard
}

fn live(guard: &WriteGuard) -> PathBuf {
    db_path(guard.app_data_dir())
}

fn make_bundle(guard: &WriteGuard, writer: &Writer) -> PathBuf {
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    PathBuf::from(create(guard, &reads, 1_000).unwrap().path)
}

/// Every table's rows, as text, sorted.
fn dump(path: &Path) -> BTreeMap<String, Vec<String>> {
    let conn = Connection::open(path).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND sql NOT LIKE 'CREATE VIRTUAL%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|t| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
            let width = stmt.column_count();
            let mut rows: Vec<String> = stmt
                .query_map([], |r| {
                    let row: Vec<rusqlite::types::Value> =
                        (0..width).map(|i| r.get(i)).collect::<Result<_, _>>()?;
                    Ok(format!("{row:?}"))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            rows.sort();
            (t, rows)
        })
        .collect()
}

fn crate_names(path: &Path) -> Vec<String> {
    dump(path)["crate"].clone()
}

/// Imports `zip` into `guard`'s app as the app would: stage, confirm, then
/// the swap at the next start.
fn import(guard: &WriteGuard, zip: &Path, now: i64) -> BundleSummary {
    let summary = stage_bundle(guard, zip).unwrap();
    confirm_staged(guard).unwrap();
    finish(guard, &Layout::of(guard), now, &mut || false).unwrap();
    summary
}

/// A zip of `entries`, written outside every app folder.
fn zip_of(path: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    path.to_path_buf()
}

fn manifest_json(format: u32) -> Vec<u8> {
    serde_json::to_vec(&Manifest {
        app: APP.to_owned(),
        format,
        app_version: "0.0.0".to_owned(),
        schema_version: 1,
        created_ms: 5,
        counts: Counts {
            tracks: 0,
            crates: 0,
            playlists: 0,
            tags: 0,
            music_folders: 0,
        },
    })
    .unwrap()
}

/// The database inside a bundle.
fn db_in(zip: &Path) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(File::open(zip).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(DB_ENTRY)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

/// A database file with this build's schema up to `upto` migrations.
fn schema_db(path: &Path, upto: usize) -> Vec<u8> {
    let mut conn = Connection::open(path).unwrap();
    migrations::run(&mut conn, &MIGRATIONS[..upto]).unwrap();
    drop(conn);
    fs::read(path).unwrap()
}

fn kept_ids(guard: &WriteGuard) -> Vec<String> {
    list_kept(guard).into_iter().map(|k| k.id).collect()
}

#[test]
fn a_round_trip_keeps_every_user_table_and_the_id_counters() {
    let root = TempDir::new().unwrap();
    let (a, writer) = library(root.path(), "a", SEED);
    let zip = make_bundle(&a, &writer);
    writer.close();
    let before = dump(&live(&a));
    let b = closed_library(root.path(), "b", "");
    let summary = import(&b, &zip, 2_000);
    let after = dump(&live(&b));

    assert_eq!(summary.counts.tracks, 1);
    assert_eq!(
        summary.app_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    let machine = [
        "job",
        "service_optin",
        "setting",
        "volume",
        "embedding_failure",
    ];
    for (table, rows) in &before {
        if !machine.contains(&table.as_str()) {
            assert_eq!(&after[table], rows, "{table} changed on the way");
        }
    }
    assert_eq!(after.len(), before.len(), "a table was added or lost");
    assert!(!after["sqlite_sequence"].is_empty());
    assert_eq!(after["crate_entry"], before["crate_entry"]);
    // The user's settings and the drive travel; only this PC's parts stay.
    let settings = after["setting"].join("\n");
    assert!(settings.contains("key_notation") && settings.contains("energy_as_stars"));
    assert!(after["volume"][0].contains("serial=FS-1234ABCD"));
    assert!(after["volume"][0].contains("Null"));
    assert!(after["job"].is_empty() && after["service_optin"].is_empty());
}

#[test]
fn a_bundle_carries_the_audio_models_fingerprints_and_not_its_failures() {
    let root = TempDir::new().unwrap();
    let (a, writer) = library(root.path(), "a", SEED);
    let zip = make_bundle(&a, &writer);
    writer.close();
    let before = dump(&live(&a));
    let b = closed_library(root.path(), "b", "");
    import(&b, &zip, 2_000);
    let after = dump(&live(&b));
    // The results are the user's own audio's, hours of work: they travel.
    assert_eq!(after["embedding"].len(), 1);
    assert_eq!(after["embedding"], before["embedding"]);
    // The files this PC couldn't hear are tried again on the other.
    assert_eq!(before["embedding_failure"].len(), 1);
    assert!(after["embedding_failure"].is_empty());
}

#[test]
fn a_bundle_carries_no_update_check_answer_so_a_new_pc_is_asked_and_sees_no_stale_notice() {
    let root = TempDir::new().unwrap();
    let (a, writer) = library(root.path(), "a", SEED);
    let bytes = db_in(&make_bundle(&a, &writer));
    let copy = root.path().join("copy.db");
    fs::write(&copy, &bytes).unwrap();
    let settings = dump(&copy)["setting"].join(
        "
",
    );
    assert!(!settings.contains("update_check_asked"));
    assert!(!settings.contains("update_check_result"));
    // The Library itself keeps its answer.
    assert!(dump(&live(&a))["setting"]
        .join(
            "
"
        )
        .contains("update_check_asked"));
}

#[test]
fn the_bundle_holds_none_of_this_pcs_settings_jobs_or_consents() {
    let root = TempDir::new().unwrap();
    let (a, writer) = library(root.path(), "a", SEED);
    let zip = make_bundle(&a, &writer);
    let bytes = db_in(&zip);
    // Not even in free pages.
    assert!(!bytes
        .windows(b"rekordbox-secret-path".len())
        .any(|w| w == b"rekordbox-secret-path"));
    let copy = root.path().join("copy.db");
    fs::write(&copy, &bytes).unwrap();
    let rows = dump(&copy);
    let settings = rows["setting"].join("\n");
    for key in MACHINE_SETTINGS {
        assert!(
            !settings.contains(&format!("\"{key}\"")),
            "{key} is in the bundle"
        );
    }
    assert!(settings.contains("rekordbox_xml_last_read") && settings.contains("readAt"));
    assert!(settings.contains("key_notation"));
    assert!(rows["job"].is_empty());
    assert!(rows["service_optin"].is_empty());
    assert!(!rows["volume"][0].contains("E:/"));
    // The Library itself is untouched.
    writer.close();
    assert!(dump(&live(&a))["setting"]
        .join("\n")
        .contains("rekordbox-secret-path"));
}

#[test]
fn inspecting_a_bundle_reads_its_counts_and_stages_nothing() {
    let root = TempDir::new().unwrap();
    let (a, writer) = library(root.path(), "a", SEED);
    let zip = make_bundle(&a, &writer);
    let summary = inspect(&a, &zip).unwrap();
    assert_eq!((summary.created_ms, summary.counts.crates), (1_000, 1));
    assert_eq!(summary.schema_version, MIGRATIONS.last().unwrap().version());
    assert!(!Layout::of(&a).pending.exists());
    assert!(matches!(
        confirm_staged(&a),
        Err(BundleError::NothingStaged)
    ));
}

/// `stage` must refuse with `kind`, leaving the live Library's bytes and
/// the import folder as they were.
fn refused(entries: &[(&str, &[u8])], caps: &Caps, kind: fn(&BundleError) -> bool) {
    let root = TempDir::new().unwrap();
    let b = closed_library(root.path(), "b", SEED);
    let before = fs::read(live(&b)).unwrap();
    let zip = zip_of(&root.path().join("picked.zip"), entries);
    let error = stage_bundle_capped(&b, &zip, caps).unwrap_err();
    assert!(kind(&error), "{error:?}");
    assert_eq!(fs::read(live(&b)).unwrap(), before);
    assert!(!Layout::of(&b).pending.exists());
    finish(&b, &Layout::of(&b), 9, &mut || false).unwrap();
    assert_eq!(fs::read(live(&b)).unwrap(), before);
}

#[test]
fn a_bundle_from_a_newer_schema_or_format_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("newer.db");
    schema_db(&path, MIGRATIONS.len());
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO schema_migration (version, name, checksum) VALUES (9999, '9999_future.sql', 'x')",
        [],
    )
    .unwrap();
    drop(conn);
    let newer = fs::read(&path).unwrap();
    let current = schema_db(&dir.path().join("current.db"), MIGRATIONS.len());
    let is_newer = |e: &BundleError| matches!(e, BundleError::Newer);
    refused(
        &[(MANIFEST_ENTRY, &manifest_json(1)), (DB_ENTRY, &newer)],
        &check::CAPS,
        is_newer,
    );
    refused(
        &[(MANIFEST_ENTRY, &manifest_json(2)), (DB_ENTRY, &current)],
        &check::CAPS,
        is_newer,
    );
}

#[test]
fn a_damaged_or_foreign_file_is_refused_and_the_library_is_unchanged() {
    let dir = TempDir::new().unwrap();
    let foreign = dir.path().join("foreign.db");
    Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE notes (x TEXT); INSERT INTO notes VALUES ('hi');")
        .unwrap();
    let foreign = fs::read(&foreign).unwrap();
    let mut garbage = b"SQLite format 3\0".to_vec();
    garbage.extend((0..8192u32).map(|i| (i * 7 % 251) as u8));
    let manifest = manifest_json(1);
    let damaged = |e: &BundleError| matches!(e, BundleError::Damaged);
    let not_ours = |e: &BundleError| matches!(e, BundleError::NotOurs);
    refused(
        &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &garbage)],
        &check::CAPS,
        damaged,
    );
    refused(
        &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &foreign)],
        &check::CAPS,
        not_ours,
    );
    refused(
        &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, b"plain text")],
        &check::CAPS,
        not_ours,
    );
    refused(
        &[(MANIFEST_ENTRY, b"{}"), (DB_ENTRY, &foreign)],
        &check::CAPS,
        not_ours,
    );
    // Not a zip at all.
    let root = TempDir::new().unwrap();
    let b = closed_library(root.path(), "b", "");
    let file = root.path().join("not.zip");
    fs::write(&file, b"PK nothing").unwrap();
    assert!(matches!(stage_bundle(&b, &file), Err(BundleError::NotOurs)));
}

#[test]
fn a_zip_with_unexpected_paths_or_entries_is_refused() {
    let dir = TempDir::new().unwrap();
    let db = schema_db(&dir.path().join("ok.db"), MIGRATIONS.len());
    let manifest = manifest_json(1);
    let not_ours = |e: &BundleError| matches!(e, BundleError::NotOurs);
    for entries in [
        vec![(MANIFEST_ENTRY, &manifest[..]), ("../library.db", &db[..])],
        vec![(MANIFEST_ENTRY, &manifest[..]), ("sub/library.db", &db[..])],
        vec![
            (MANIFEST_ENTRY, &manifest[..]),
            (DB_ENTRY, &db[..]),
            ("notes.txt", b"x"),
        ],
        vec![(DB_ENTRY, &db[..])],
    ] {
        refused(&entries, &check::CAPS, not_ours);
    }
}

#[test]
fn an_entry_over_its_cap_is_refused() {
    let dir = TempDir::new().unwrap();
    let db = schema_db(&dir.path().join("ok.db"), MIGRATIONS.len());
    let manifest = manifest_json(1);
    let too_large = |e: &BundleError| matches!(e, BundleError::TooLarge);
    let entries = [(MANIFEST_ENTRY, &manifest[..]), (DB_ENTRY, &db[..])];
    let small_db = Caps {
        db: 4096,
        manifest: 64 << 10,
    };
    refused(&entries, &small_db, too_large);
    let small_manifest = Caps {
        db: 1 << 30,
        manifest: 8,
    };
    refused(&entries, &small_manifest, too_large);
}

#[test]
fn an_older_schema_bundle_is_migrated_on_the_staged_copy() {
    let root = TempDir::new().unwrap();
    let old = schema_db(&root.path().join("old.db"), MIGRATIONS.len() - 3);
    let zip = zip_of(
        &root.path().join("old.zip"),
        &[(MANIFEST_ENTRY, &manifest_json(1)), (DB_ENTRY, &old)],
    );
    let b = closed_library(root.path(), "b", SEED);
    let before = fs::read(live(&b)).unwrap();
    let summary = stage_bundle(&b, &zip).unwrap();
    assert_eq!(summary.schema_version, MIGRATIONS.last().unwrap().version());
    let staged = Connection::open(Layout::of(&b).pending).unwrap();
    assert_eq!(
        migrations::applied(&staged).unwrap().len(),
        MIGRATIONS.len()
    );
    drop(staged);
    assert_eq!(fs::read(live(&b)).unwrap(), before);
}

/// An app with Library "old" and a staged, confirmed Library "new".
fn confirmed_swap(root: &Path) -> WriteGuard {
    let guard = closed_library(
        root,
        "app",
        "INSERT INTO crate (id, kind, name) VALUES (1, 'static', 'old');",
    );
    let new = closed_library(
        root,
        "other",
        "INSERT INTO crate (id, kind, name) VALUES (1, 'static', 'new');",
    );
    let layout = Layout::of(&guard);
    fs::create_dir_all(&layout.import).unwrap();
    fs::copy(live(&new), &layout.pending).unwrap();
    confirm_staged(&guard).unwrap();
    guard
}

fn has_crate(path: &Path, name: &str) -> bool {
    crate_names(path)
        .iter()
        .any(|row| row.contains(&format!("\"{name}\"")))
}

#[test]
fn a_crash_at_any_swap_step_recovers_to_one_whole_library() {
    let mut crash_at = 0;
    loop {
        let root = TempDir::new().unwrap();
        let guard = confirmed_swap(root.path());
        let layout = Layout::of(&guard);
        let mut asked = 0;
        let result = finish(&guard, &layout, 100, &mut || {
            asked += 1;
            asked == crash_at + 1
        });
        if result.is_ok() {
            assert!(crash_at >= 4, "only {crash_at} steps");
            break;
        }
        assert!(matches!(result, Err(BundleError::Interrupted)));
        // The old Library is whole somewhere: in place, or moved aside.
        let aside = layout.swapping.join(DB_FILE_NAME);
        assert!(
            live(&guard).exists() || aside.exists(),
            "step {crash_at}: no Library"
        );
        // The next start finishes the swap.
        finish(&guard, &layout, 200, &mut || false).unwrap();
        assert!(has_crate(&live(&guard), "new") && !has_crate(&live(&guard), "old"));
        let kept = kept_ids(&guard);
        assert_eq!(kept.len(), 1, "step {crash_at}: {kept:?}");
        assert!(has_crate(
            &layout.backups.join(&kept[0]).join(DB_FILE_NAME),
            "old"
        ));
        assert!(!layout.marker.exists() && !layout.pending.exists() && !layout.swapping.exists());
        crash_at += 1;
    }
}

#[test]
fn the_old_librarys_log_moves_with_it_and_never_joins_the_new_file() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    let live = live(&guard);
    write_only_to_the_log(root.path(), &live);
    // A swap under way: the log already moved aside, the file not yet.
    log_moved_aside(&layout, &live);
    finish(&guard, &layout, 300, &mut || false).unwrap();
    assert!(!swap_piece(&live, "-wal").exists());
    assert!(has_crate(&live, "new") && !has_crate(&live, "in-log"));
    let kept = layout.backups.join(&kept_ids(&guard)[0]).join(DB_FILE_NAME);
    assert!(has_crate(&kept, "in-log") && has_crate(&kept, "old"));
}

/// The state of a swap stopped after the log moved aside and before the file.
fn log_moved_aside(layout: &Layout, live: &Path) {
    fs::create_dir_all(&layout.swapping).unwrap();
    let aside = layout.swapping.join(DB_FILE_NAME);
    fs::rename(swap_piece(live, "-wal"), swap_piece(&aside, "-wal")).unwrap();
}

/// Gives the Library at `live` a crate "in-log" that is only in its -wal,
/// as a crash leaves it.
fn write_only_to_the_log(root: &Path, live: &Path) {
    let side = root.join("side");
    fs::create_dir_all(&side).unwrap();
    let conn = Connection::open(live).unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute(
        "INSERT INTO crate (id, kind, name) VALUES (9, 'static', 'in-log')",
        [],
    )
    .unwrap();
    fs::copy(live, side.join("db")).unwrap();
    fs::copy(swap_piece(live, "-wal"), side.join("wal")).unwrap();
    drop(conn);
    fs::copy(side.join("db"), live).unwrap();
    fs::copy(side.join("wal"), swap_piece(live, "-wal")).unwrap();
    assert!(swap_piece(live, "-wal").exists());
}

fn swap_piece(base: &Path, suffix: &str) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[test]
fn an_unconfirmed_staged_library_is_dropped_at_start() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    fs::remove_file(&layout.marker).unwrap();
    assert_eq!(
        finish(&guard, &layout, 1, &mut || false).unwrap(),
        Swapped::Nothing
    );
    assert!(has_crate(&live(&guard), "old"));
    assert!(!layout.pending.exists() && kept_ids(&guard).is_empty());
}

#[test]
fn the_replaced_library_is_kept_and_can_be_restored() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    let Swapped::Replaced(Some(first)) = finish(&guard, &layout, 1_000, &mut || false).unwrap()
    else {
        panic!("not replaced")
    };
    let kept = list_kept(&guard);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (kept[0].created_ms, kept[0].counts.unwrap().crates),
        (1_000, 1)
    );
    assert!(matches!(
        stage_kept(&guard, "nope"),
        Err(BundleError::NoSuchBackup)
    ));
    let summary = stage_kept(&guard, &first).unwrap();
    assert_eq!(summary.created_ms, 1_000);
    confirm_staged(&guard).unwrap();
    finish(&guard, &layout, 2_000, &mut || false).unwrap();
    assert!(has_crate(&live(&guard), "old") && !has_crate(&live(&guard), "new"));
    // The Library it replaced is kept too, newest first.
    assert_eq!(kept_ids(&guard), [stamp_of(2_000), first]);
    assert!(has_crate(
        &layout.backups.join(stamp_of(2_000)).join(DB_FILE_NAME),
        "new"
    ));
}

fn stamp_of(ms: i64) -> String {
    format!("{ms:015}")
}

#[test]
fn only_the_newest_three_backups_are_kept() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    let other = live(&WriteGuard::app_data(&root.path().join("other")).unwrap());
    for now in 1..=KEEP_BACKUPS as i64 + 2 {
        if now > 1 {
            fs::copy(&other, &layout.pending).unwrap();
            confirm_staged(&guard).unwrap();
        }
        finish(&guard, &layout, now, &mut || false).unwrap();
    }
    assert_eq!(kept_ids(&guard), [stamp_of(5), stamp_of(4), stamp_of(3)]);
}

#[test]
fn bundles_and_swaps_write_nothing_outside_the_app_folders() {
    let root = TempDir::new().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let (a, writer) = library(root.path(), "a", SEED);
    let b = closed_library(root.path(), "b", "");
    let picked = outside.join("picked.zip");
    fs::copy(make_bundle(&a, &writer), &picked).unwrap();
    writer.close();
    let apps = [root.path().join("a"), root.path().join("b")];
    let skip: Vec<&Path> = apps.iter().map(PathBuf::as_path).collect();
    let mut before = BTreeMap::new();
    snapshot(root.path(), &skip, &mut before);

    inspect(&b, &picked).unwrap();
    import(&b, &picked, 10);
    let id = kept_ids(&b).pop().unwrap();
    stage_kept(&b, &id).unwrap();
    confirm_staged(&b).unwrap();
    finish(&b, &Layout::of(&b), 20, &mut || false).unwrap();

    let mut after = BTreeMap::new();
    snapshot(root.path(), &skip, &mut after);
    assert_eq!(before, after);
}

/// A current-schema database at `path` changed by `break_it`, as bytes.
fn broken_db(path: &Path, break_it: impl FnOnce(&Connection, &Path)) -> Vec<u8> {
    schema_db(path, MIGRATIONS.len());
    let conn = Connection::open(path).unwrap();
    break_it(&conn, path);
    drop(conn);
    fs::read(path).unwrap()
}

#[test]
fn a_bundle_failing_only_its_integrity_or_foreign_key_check_is_refused() {
    let dir = TempDir::new().unwrap();
    let mut torn = broken_db(&dir.path().join("torn.db"), |c, _| {
        c.execute_batch("CREATE TABLE zz (x); INSERT INTO zz VALUES (1);")
            .unwrap();
    });
    // Tear the page of a table nothing else reads.
    let page = {
        let c = Connection::open(dir.path().join("torn.db")).unwrap();
        let root: i64 = c
            .query_row(
                "SELECT rootpage FROM sqlite_master WHERE name = 'zz'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let size: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        usize::try_from((root - 1) * size).unwrap()
    };
    torn[page] = 0xFF;
    let dangling = broken_db(&dir.path().join("dangling.db"), |c, _| {
        c.execute_batch(
            "PRAGMA foreign_keys = OFF;
             INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (99, 'DJ', 'dj');",
        )
        .unwrap();
    });
    let edited = broken_db(&dir.path().join("edited.db"), |c, _| {
        c.execute(
            "UPDATE schema_migration SET checksum = 'x' WHERE version = 2",
            [],
        )
        .unwrap();
    });
    let manifest = manifest_json(1);
    let damaged = |e: &BundleError| matches!(e, BundleError::Damaged);
    refused(
        &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &torn)],
        &check::CAPS,
        damaged,
    );
    refused(
        &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &dangling)],
        &check::CAPS,
        damaged,
    );
    let not_ours = |e: &BundleError| matches!(e, BundleError::NotOurs);
    refused(
        &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &edited)],
        &check::CAPS,
        not_ours,
    );
}

#[test]
fn staging_again_withdraws_an_earlier_confirm() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let db = schema_db(&root.path().join("ok.db"), MIGRATIONS.len());
    let zip = zip_of(
        &root.path().join("again.zip"),
        &[(MANIFEST_ENTRY, &manifest_json(1)), (DB_ENTRY, &db)],
    );
    stage_bundle(&guard, &zip).unwrap();
    assert!(!Layout::of(&guard).marker.exists());
    finish(&guard, &Layout::of(&guard), 1, &mut || false).unwrap();
    assert!(has_crate(&live(&guard), "old"));
}

#[test]
fn only_the_newest_bundle_is_kept_and_never_the_one_just_made() {
    let root = TempDir::new().unwrap();
    let (a, writer) = library(root.path(), "a", "");
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    let folder = a.app_data_dir().join("bundles");
    for now in [10, 20, 30] {
        create(&a, &reads, now).unwrap();
    }
    let name = |n: i64| format!("tracklist-pro-{}.zip", stamp_of(n));
    assert_eq!(folders::ours(&folder, |_| true), [name(30)]);
    // The clock went back: the bundle just made is still the one kept.
    let made = create(&a, &reads, 5).unwrap();
    assert!(Path::new(&made.path).exists());
    assert_eq!(folders::ours(&folder, |_| true), [name(5)]);
}

#[test]
fn a_backup_made_while_the_clock_is_behind_is_kept() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    for ms in [100, 200, 300] {
        fs::create_dir_all(layout.backups.join(stamp_of(ms))).unwrap();
        fs::write(layout.backups.join(stamp_of(ms)).join(DB_FILE_NAME), b"x").unwrap();
    }
    let swapped = finish(&guard, &layout, 50, &mut || false).unwrap();
    assert_eq!(swapped, Swapped::Replaced(Some(stamp_of(50))));
    assert_eq!(
        kept_ids(&guard),
        [stamp_of(300), stamp_of(200), stamp_of(50)]
    );
    assert!(has_crate(
        &layout.backups.join(stamp_of(50)).join(DB_FILE_NAME),
        "old"
    ));
}

#[test]
fn a_backup_name_already_taken_moves_to_the_next_free_one() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    fs::create_dir_all(layout.backups.join(stamp_of(7))).unwrap();
    let swapped = finish(&guard, &layout, 7, &mut || false).unwrap();
    assert_eq!(swapped, Swapped::Replaced(Some(stamp_of(8))));
}

/// The step `finish` asks about last before a crash at `n` (1-based).
fn crash_at(n: usize) -> impl FnMut() -> bool {
    let mut asked = 0;
    move || {
        asked += 1;
        asked == n
    }
}

#[test]
fn a_swap_stopped_before_the_new_file_moved_in_opens_the_old_library_once() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    // Steps: make the folder, move the file aside, move the new one in.
    let swapped = start(&guard, &layout, 1, &mut crash_at(3)).unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
    assert!(has_crate(&live(&guard), "old"));
    // The confirm is withdrawn: the next start doesn't try again.
    assert!(!layout.marker.exists());
    assert_eq!(
        start(&guard, &layout, 2, &mut || false).unwrap(),
        Swapped::Nothing
    );
    assert!(has_crate(&live(&guard), "old") && !layout.pending.exists());
    assert!(kept_ids(&guard).is_empty());
}

#[test]
fn a_log_left_aside_is_folded_in_even_when_the_swap_stops_right_after() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    let live = live(&guard);
    write_only_to_the_log(root.path(), &live);
    // Under way already: the -wal moved aside, then the stop before the file.
    log_moved_aside(&layout, &live);
    let swapped = start(&guard, &layout, 1, &mut crash_at(3)).unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
    assert!(!swap_piece(&layout.swapping.join(DB_FILE_NAME), "-wal").exists());
    assert!(has_crate(&live, "in-log") && has_crate(&live, "old"));
}

#[test]
fn a_swap_stopped_between_pieces_brings_the_piece_home() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    let live = live(&guard);
    // A file the fold can't open, with shared memory beside it: it moves aside
    // (step 2), and the stop before the file (step 3) leaves it there.
    fs::write(&live, b"not a database").unwrap();
    fs::write(swap_piece(&live, "-shm"), b"shm").unwrap();
    let swapped = start(&guard, &layout, 1, &mut crash_at(3)).unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
    // The start's own put-back brought it home beside its file.
    assert_eq!(fs::read(swap_piece(&live, "-shm")).unwrap(), b"shm");
    assert!(!swap_piece(&layout.swapping.join(DB_FILE_NAME), "-shm").exists());
    assert_eq!(fs::read(&live).unwrap(), b"not a database");
}

/// A confirmed swap whose old Library has a crate "in-log" only in its log,
/// and an earlier stop's `swapping` folder with `stale` in it.
fn swap_over_a_stale_folder(root: &Path, stale: &[&str]) -> (WriteGuard, Layout) {
    let guard = confirmed_swap(root);
    let layout = Layout::of(&guard);
    write_only_to_the_log(root, &live(&guard));
    fs::create_dir_all(&layout.swapping).unwrap();
    for name in stale {
        fs::write(layout.swapping.join(name), b"x").unwrap();
    }
    (guard, layout)
}

#[test]
fn an_empty_set_aside_folder_left_by_a_stop_doesnt_skip_folding_the_log_in() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = swap_over_a_stale_folder(root.path(), &[]);
    finish(&guard, &layout, 1, &mut || false).unwrap();
    let kept = layout.backups.join(&kept_ids(&guard)[0]).join(DB_FILE_NAME);
    assert!(!swap_piece(&kept, "-wal").exists() && !swap_piece(&kept, "-shm").exists());
    assert!(has_crate(&kept, "in-log") && has_crate(&kept, "old"));
    assert!(has_crate(&live(&guard), "new"));
}

#[test]
fn a_log_left_in_the_set_aside_folder_without_its_file_comes_home_and_is_folded_in() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = swap_over_a_stale_folder(root.path(), &[]);
    log_moved_aside(&layout, &live(&guard));
    finish(&guard, &layout, 1, &mut || false).unwrap();
    let kept = layout.backups.join(&kept_ids(&guard)[0]).join(DB_FILE_NAME);
    assert!(!swap_piece(&kept, "-wal").exists() && !swap_piece(&kept, "-shm").exists());
    assert!(has_crate(&kept, "in-log") && has_crate(&kept, "old"));
    assert!(has_crate(&live(&guard), "new") && !layout.swapping.exists());
}

#[test]
fn a_stale_piece_that_clashes_with_one_in_place_is_refused_and_both_are_kept() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = swap_over_a_stale_folder(root.path(), &["tracklist-pro.db-wal"]);
    let started = start(&guard, &layout, 1, &mut || false).unwrap();
    assert_eq!(started, Swapped::NotReplaced(EventReason::FileInTheWay));
    let live = live(&guard);
    assert!(has_crate(&live, "old") && has_crate(&live, "in-log"));
    // Kept whole, set apart where nothing reads it.
    let apart = layout.backups.join(format!("{}-stray", stamp(1)));
    assert_eq!(fs::read(apart.join("tracklist-pro.db-wal")).unwrap(), b"x");
    assert!(!layout.swapping.exists());
}

#[test]
fn a_stale_log_refused_for_its_clash_is_not_replayed_when_the_import_is_confirmed_again() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = swap_over_a_stale_folder(root.path(), &[]);
    let live = live(&guard);
    // An older copy of the Library's log sits in `swapping`, its name taken
    // beside the Library: the swap is refused and nothing is put back.
    let wal = layout.swapping.join("tracklist-pro.db-wal");
    fs::copy(swap_piece(&live, "-wal"), &wal).unwrap();
    let started = start(&guard, &layout, 1, &mut || false).unwrap();
    assert_eq!(started, Swapped::NotReplaced(EventReason::FileInTheWay));
    // The Library carries on and is written to.
    write_later(&live);
    // Confirmed again in the same session: the old log must not come back.
    confirm_staged(&guard).unwrap();
    let swapped = start(&guard, &layout, 2, &mut crash_at(2)).unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
    assert!(has_crate(&live, "later") && has_crate(&live, "in-log"));
    let apart = layout.backups.join(format!("{}-stray", stamp(1)));
    assert!(apart.join("tracklist-pro.db-wal").exists());
}

/// The crates in the Library at `live`, after adding "later" to it (closed
/// again, so its log is written into the file).
fn write_later(live: &Path) {
    let conn = Connection::open(live).unwrap();
    conn.execute(
        "INSERT INTO crate (id, kind, name) VALUES (20, 'static', 'later')",
        [],
    )
    .unwrap();
}

#[test]
fn a_log_that_couldnt_come_home_is_set_apart_and_never_replayed_over_later_writes() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = swap_over_a_stale_folder(root.path(), &[]);
    let live = live(&guard);
    log_moved_aside(&layout, &live);
    // A stop right after the log moved, then a put-back that fails: the
    // Library opens without the log, which must not come back later.
    let swapped = start_with(&guard, &layout, 1, &mut crash_at(1), &mut |_, _| {
        Err(BundleError::Interrupted)
    })
    .unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
    let apart = layout.backups.join(format!("{}-stray", stamp(1)));
    assert!(swap_piece(&apart.join(DB_FILE_NAME), "-wal").exists());
    assert!(!layout.swapping.exists());
    write_later(&live);
    // The staged file is still there: confirmed again, it swaps in.
    confirm_staged(&guard).unwrap();
    let swapped = start(&guard, &layout, 2, &mut crash_at(2)).unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
    assert!(has_crate(&live, "later") && has_crate(&live, "old"));
    assert!(!has_crate(&live, "in-log"));
    assert!(swap_piece(&apart.join(DB_FILE_NAME), "-wal").exists());
    // And once it goes through, the backup is the Library as it was left.
    confirm_staged(&guard).unwrap();
    start(&guard, &layout, 3, &mut || false).unwrap();
    let kept = layout.backups.join(&kept_ids(&guard)[0]).join(DB_FILE_NAME);
    assert!(has_crate(&kept, "later") && !has_crate(&kept, "in-log"));
    assert!(swap_piece(&apart.join(DB_FILE_NAME), "-wal").exists());
}

#[test]
fn pieces_left_without_their_file_while_the_library_is_in_place_are_set_apart_at_start() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = set_aside_whole(root.path());
    // The Library is home; what stays in `swapping` has no file.
    fs::rename(layout.swapping.join(DB_FILE_NAME), live(&guard)).unwrap();
    write_later(&live(&guard));
    assert!(swap_piece(&layout.swapping.join(DB_FILE_NAME), "-wal").exists());
    // Two starts: the second finds a name taken and keeps both.
    for (now, name) in [(5, 5), (5, 6)] {
        fs::create_dir_all(&layout.swapping).unwrap();
        fs::write(layout.swapping.join("piece"), name.to_string()).unwrap();
        assert_eq!(
            start(&guard, &layout, now, &mut || false).unwrap(),
            Swapped::Nothing
        );
    }
    assert!(has_crate(&live(&guard), "later"));
    let first = layout.backups.join(format!("{}-stray", stamp(5)));
    let second = layout.backups.join(format!("{}-stray", stamp(6)));
    assert!(swap_piece(&first.join(DB_FILE_NAME), "-wal").exists());
    assert_eq!(fs::read(second.join("piece")).unwrap(), b"6");
    assert!(!layout.swapping.exists() && kept_ids(&guard).is_empty());
}

#[test]
fn a_swap_done_but_not_tidied_opens_the_new_library_and_keeps_the_old() {
    for stop in [4, 5] {
        // 4: before the old Library is named a backup; 5: before the marker goes.
        let root = TempDir::new().unwrap();
        let guard = confirmed_swap(root.path());
        let layout = Layout::of(&guard);
        let swapped = start(&guard, &layout, 1, &mut crash_at(stop)).unwrap();
        assert_eq!(swapped, Swapped::NotReplaced(EventReason::CantWrite));
        assert!(has_crate(&live(&guard), "new"), "stop {stop}");
        assert!(!layout.marker.exists());
        // The next start keeps what was set aside as a backup.
        assert_eq!(
            start(&guard, &layout, 2, &mut || false).unwrap(),
            Swapped::Nothing
        );
        let kept = kept_ids(&guard);
        assert_eq!(kept.len(), 1, "stop {stop}");
        assert!(has_crate(
            &layout.backups.join(&kept[0]).join(DB_FILE_NAME),
            "old"
        ));
        assert!(!layout.swapping.exists());
    }
}

#[test]
fn a_file_in_the_way_is_never_written_over_and_is_logged_once() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    let events = EventLog::open(&guard);
    fs::create_dir_all(&layout.swapping).unwrap();
    let aside = layout.swapping.join(DB_FILE_NAME);
    fs::write(&aside, b"someone's file").unwrap();
    let swapped = on_startup(&guard, &events).unwrap();
    assert_eq!(swapped, Swapped::NotReplaced(EventReason::FileInTheWay));
    assert_eq!(fs::read(&aside).unwrap(), b"someone's file");
    assert!(has_crate(&live(&guard), "old"));
    let logged = events.events();
    assert!(logged
        .iter()
        .any(|e| e.kind == EventKind::LibraryNotReplaced
            && e.reason == Some(EventReason::FileInTheWay)));
    // The next start doesn't meet it again: the file is kept as a backup.
    assert_eq!(on_startup(&guard, &events).unwrap(), Swapped::Nothing);
    let kept = kept_ids(&guard);
    assert_eq!(
        fs::read(layout.backups.join(&kept[0]).join(DB_FILE_NAME)).unwrap(),
        b"someone's file"
    );
    assert_eq!(events.events().len(), logged.len());
}

#[test]
fn startup_fails_only_with_no_library_in_place() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    // No old Library, and the new one never moves in.
    fs::remove_file(live(&guard)).unwrap();
    let failed = start(&guard, &layout, 1, &mut crash_at(2));
    assert!(matches!(failed, Err(BundleError::Interrupted)));
    assert!(!live(&guard).exists() && layout.marker.exists());
    // The next start finishes it.
    start(&guard, &layout, 2, &mut || false).unwrap();
    assert!(has_crate(&live(&guard), "new"));
}

#[test]
fn the_old_librarys_log_is_folded_into_its_backup() {
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    write_only_to_the_log(root.path(), &live(&guard));
    finish(&guard, &layout, 1, &mut || false).unwrap();
    let kept = layout.backups.join(&kept_ids(&guard)[0]).join(DB_FILE_NAME);
    assert!(!swap_piece(&kept, "-wal").exists() && !swap_piece(&kept, "-shm").exists());
    assert!(has_crate(&kept, "in-log") && has_crate(&kept, "old"));
}

#[test]
fn a_database_copy_is_written_only_to_a_staging_file() {
    let root = TempDir::new().unwrap();
    let (guard, writer) = library(root.path(), "a", SEED);
    let from = Connection::open_in_memory().unwrap();
    for path in [
        live(&guard),
        guard.app_data_dir().join("copy.db"),
        guard
            .app_data_dir()
            .join("staging")
            .join("x")
            .join(DB_FILE_NAME),
    ] {
        let checked = guard.check(&path).unwrap();
        assert!(checked.copy_database_from(&from).is_err(), "{path:?}");
    }
    writer.close();
    assert!(has_crate(&live(&guard), "Warmup"));
}

#[test]
fn inspecting_a_newer_bundle_is_refused_as_newer() {
    let root = TempDir::new().unwrap();
    let a = closed_library(root.path(), "a", "");
    let path = root.path().join("newer.db");
    schema_db(&path, MIGRATIONS.len());
    Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO schema_migration (version, name, checksum) VALUES (9999, '9999_future.sql', 'x')",
            [],
        )
        .unwrap();
    let zip = zip_of(
        &root.path().join("newer.zip"),
        &[
            (MANIFEST_ENTRY, &manifest_json(1)),
            (DB_ENTRY, &fs::read(&path).unwrap()),
        ],
    );
    assert!(matches!(inspect(&a, &zip), Err(BundleError::Newer)));
}

#[test]
fn a_bundle_whose_schema_differs_from_its_migrations_is_refused() {
    let dir = TempDir::new().unwrap();
    let manifest = manifest_json(1);
    let not_ours = |e: &BundleError| matches!(e, BundleError::NotOurs);
    for (name, edit) in [
        ("dropped.db", "DROP TABLE ui_setting;"),
        ("extra.db", "CREATE INDEX extra ON setting (value);"),
        ("altered.db", "ALTER TABLE job ADD COLUMN extra TEXT;"),
    ] {
        let db = broken_db(&dir.path().join(name), |c, _| {
            c.execute_batch(edit).unwrap()
        });
        refused(
            &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &db)],
            &check::CAPS,
            not_ours,
        );
        let zip = zip_of(
            &dir.path().join(format!("{name}.zip")),
            &[(MANIFEST_ENTRY, &manifest), (DB_ENTRY, &db)],
        );
        let root = TempDir::new().unwrap();
        let b = closed_library(root.path(), "b", "");
        assert!(
            matches!(inspect(&b, &zip), Err(BundleError::NotOurs)),
            "{name}"
        );
    }
}

#[test]
fn save_as_takes_only_a_bundle_name_and_refusals_reach_the_screen_as_their_own_kinds() {
    let data = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let guard = WriteGuard::app_data(data.path()).unwrap();
    let keep = KeepOut::default();
    let dest = out.path().join("a.zip");
    for name in ["../x.zip", "notes.zip", "tracklist-pro-1.zip", "a/b.zip"] {
        let refused = save_as(&guard, name, &dest, &keep).unwrap_err();
        assert!(matches!(
            refused,
            BundleError::Guard(GuardError::SaveRefused {
                why: SaveRefusal::SourceNotABundle,
                ..
            })
        ));
        assert_eq!(IpcError::from(refused).kind(), ErrorKind::SaveNotABundle);
    }
    assert!(!dest.exists());
    // A refusal from the guard keeps its own kind.
    let bundles = data.path().join("bundles");
    fs::create_dir_all(&bundles).unwrap();
    let name = &format!("tracklist-pro-{}.zip", stamp(1_794_000_000_000));
    fs::write(bundles.join(name), b"zip").unwrap();
    let refused = save_as(&guard, name, &out.path().join("a.txt"), &keep).unwrap_err();
    assert_eq!(IpcError::from(refused).kind(), ErrorKind::SaveNotZip);
    fs::write(&dest, b"mine").unwrap();
    let refused = save_as(&guard, name, &dest, &keep).unwrap_err();
    assert_eq!(IpcError::from(refused).kind(), ErrorKind::SaveExists);
    assert_eq!(fs::read(&dest).unwrap(), b"mine");
}

/// A Library set aside whole (file, log and shared memory), none at home.
fn set_aside_whole(root: &Path) -> (WriteGuard, Layout) {
    let guard = closed_library(root, "app", "");
    let layout = Layout::of(&guard);
    fs::create_dir_all(&layout.swapping).unwrap();
    let aside = layout.swapping.join(DB_FILE_NAME);
    fs::rename(live(&guard), &aside).unwrap();
    fs::write(swap_piece(&aside, "-wal"), b"wal").unwrap();
    fs::write(swap_piece(&aside, "-shm"), b"shm").unwrap();
    (guard, layout)
}

/// What sits beside the Library's place: its file and any pieces.
fn beside_live(layout: &Layout) -> Vec<&'static str> {
    ["-journal", "-wal", "-shm", ""]
        .into_iter()
        .filter(|s| swap_piece(&layout.live, s).exists())
        .collect()
}

#[test]
fn a_put_back_failing_at_any_step_leaves_no_piece_beside_a_missing_file() {
    // Moves, in order: -wal, -shm, the file. Then, with the way back working
    // or not, the pieces already home must not stay beside no file.
    for fail_at in 1..=3 {
        for way_back_works in [true, false] {
            let root = TempDir::new().unwrap();
            let (guard, layout) = set_aside_whole(root.path());
            let mut calls = 0;
            let failed = put_back_with(&guard, &layout, &mut |from, to| {
                calls += 1;
                if calls == fail_at || (calls > fail_at && !way_back_works) {
                    return Err(BundleError::Interrupted);
                }
                fs::rename(from, to).map_err(BundleError::Io)
            });
            let case = format!("fail at {fail_at}, way back works {way_back_works}");
            assert!(failed.is_err(), "{case}");
            assert!(beside_live(&layout).is_empty(), "{case}");
            // The file is never lost, and what could go back did.
            let aside = layout.swapping.join(DB_FILE_NAME);
            assert!(aside.exists(), "{case}");
            if way_back_works {
                assert!(swap_piece(&aside, "-wal").exists(), "{case}");
                assert!(swap_piece(&aside, "-shm").exists(), "{case}");
            } else if fail_at > 1 {
                // The piece that couldn't go back is kept, set aside in place.
                let stray = swap_piece(&swap_piece(&layout.live, "-wal"), ".stray");
                assert_eq!(fs::read(stray).unwrap(), b"wal", "{case}");
            }
        }
    }
}

#[test]
fn a_second_failure_keeps_the_earlier_stray_copy_and_adds_its_own() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = set_aside_whole(root.path());
    let stray = |n: &str| swap_piece(&swap_piece(&layout.live, "-wal"), n);
    fs::write(stray(".stray"), b"first").unwrap();
    fs::write(stray(".stray-2"), b"second").unwrap();
    let mut calls = 0;
    // The -wal and -shm come home, the file fails, and nothing goes back.
    let failed = put_back_with(&guard, &layout, &mut |from, to| {
        calls += 1;
        if calls >= 3 {
            return Err(BundleError::Interrupted);
        }
        fs::rename(from, to).map_err(BundleError::Io)
    });
    assert!(failed.is_err());
    assert_eq!(fs::read(stray(".stray")).unwrap(), b"first");
    assert_eq!(fs::read(stray(".stray-2")).unwrap(), b"second");
    assert_eq!(fs::read(stray(".stray-3")).unwrap(), b"wal");
    assert!(beside_live(&layout).is_empty());
}

#[test]
fn a_put_back_that_works_brings_the_whole_set_home() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = set_aside_whole(root.path());
    put_back_with(&guard, &layout, &mut |from, to| {
        fs::rename(from, to).map_err(BundleError::Io)
    })
    .unwrap();
    assert_eq!(beside_live(&layout), ["-wal", "-shm", ""]);
}

#[cfg(windows)]
#[test]
fn a_confirm_that_cant_be_deleted_still_opens_the_library_and_doesnt_loop() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = TempDir::new().unwrap();
    let guard = confirmed_swap(root.path());
    let layout = Layout::of(&guard);
    // A file in the way fails the swap; the confirm is held open (no
    // sharing), so it can't be withdrawn either.
    fs::create_dir_all(&layout.swapping).unwrap();
    let aside = layout.swapping.join(DB_FILE_NAME);
    fs::write(&aside, b"someone's file").unwrap();
    let held = File::options()
        .read(true)
        .share_mode(0)
        .open(&layout.marker)
        .unwrap();
    for now in [1, 2, 3] {
        let swapped = start(&guard, &layout, now, &mut || false).unwrap();
        assert_eq!(swapped, Swapped::NotReplaced(EventReason::FileInTheWay));
        assert!(has_crate(&live(&guard), "old"));
        assert_eq!(fs::read(&aside).unwrap(), b"someone's file");
    }
    assert!(kept_ids(&guard).is_empty());
    // Once it can be deleted, the next start settles it.
    drop(held);
    assert_eq!(
        start(&guard, &layout, 4, &mut || false).unwrap(),
        Swapped::NotReplaced(EventReason::FileInTheWay)
    );
    assert!(!layout.marker.exists());
    assert_eq!(
        start(&guard, &layout, 5, &mut || false).unwrap(),
        Swapped::Nothing
    );
}

#[test]
fn with_no_confirm_and_the_library_still_set_aside_the_next_start_brings_it_home() {
    let root = TempDir::new().unwrap();
    let (guard, layout) = set_aside_whole(root.path());
    assert!(!layout.marker.exists());
    assert_eq!(
        start(&guard, &layout, 1, &mut || false).unwrap(),
        Swapped::Nothing
    );
    assert_eq!(beside_live(&layout), ["-wal", "-shm", ""]);
}
