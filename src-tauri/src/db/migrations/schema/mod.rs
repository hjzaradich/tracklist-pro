//! Tests for the Phase 1 schema (ROADMAP §2): each migration applies on top
//! of the ones before it, and each constraint refuses the rows it exists to
//! refuse and accepts the ones it must allow. Test names state the rule.
//!
//! Every test uses a fresh database in a temp dir, never the real one.

mod analysis_never_conflicts;
mod file_quality;
mod file_roles;
mod file_stage;
mod file_verdict;
mod files;
mod fingerprint_audio_hash;
mod fingerprint_match;
mod fingerprint_matched;
mod fingerprint_rev;
mod grouping;
mod library;
mod library_upkeep;
mod partial_hash;
mod search_index;
mod sent_playlist;
mod sync_base_ids;
mod tracks;
mod workspace;

use rusqlite::{Connection, ErrorCode};

use super::{applied, run, MIGRATIONS};

/// A fresh database with the first `version` real migrations applied and
/// foreign keys on, as the writer leaves it.
pub(super) fn db_at(version: usize) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, &MIGRATIONS[..version]).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    (dir, conn)
}

/// A fresh database with every migration applied.
pub(super) fn db() -> (tempfile::TempDir, Connection) {
    db_at(MIGRATIONS.len())
}

/// Runs `sql` and returns the id of the row it inserted.
pub(super) fn insert(conn: &Connection, sql: &str) -> i64 {
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("refused: {sql}\n{e}"));
    conn.last_insert_rowid()
}

/// Asserts that SQLite runs `sql`.
pub(super) fn accepts(conn: &Connection, sql: &str) {
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("refused: {sql}\n{e}"));
}

/// Asserts that a constraint refuses `sql`, with a message containing
/// `why` (e.g. `CHECK`, `UNIQUE`, `FOREIGN KEY`, or a trigger's message),
/// and that nothing of it was kept.
pub(super) fn refuses(conn: &Connection, sql: &str, why: &str) {
    let rows_before = row_counts(conn);
    match conn.execute_batch(sql) {
        Ok(()) => panic!("accepted: {sql}"),
        Err(rusqlite::Error::SqliteFailure(e, message)) => {
            assert_eq!(
                e.code,
                ErrorCode::ConstraintViolation,
                "{sql}: failed for another reason: {message:?}"
            );
            let message = message.unwrap_or_default();
            assert!(
                message.contains(why),
                "{sql}: refused with {message:?}, expected {why:?}"
            );
        }
        Err(e) => panic!("{sql}: failed for another reason: {e}"),
    }
    // Rows, not `total_changes`: a trigger's writes count there even when
    // the statement is undone afterwards (a foreign key is checked after
    // its triggers have run).
    assert_eq!(row_counts(conn), rows_before, "{sql}: partly kept");
}

/// How many rows every table holds.
fn row_counts(conn: &Connection) -> Vec<(String, i64)> {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let rows = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
                    r.get(0)
                })
                .unwrap();
            (table, rows)
        })
        .collect()
}

/// A value from a one-row, one-column query.
pub(super) fn one<T: rusqlite::types::FromSql>(conn: &Connection, sql: &str) -> T {
    conn.query_row(sql, [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The column names of `table`.
pub(super) fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_xinfo(?1)")
        .unwrap();
    let names = stmt
        .query_map([table], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap();
    assert!(!names.is_empty(), "no table {table}");
    names
}

/// Every table in the database, sorted.
fn tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    let names = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap();
    names
}

/// The tables each migration adds, in order (ROADMAP §2 "When tables
/// arrive", Phase 0–1).
const TABLES_BY_MIGRATION: &[(&str, &[&str])] = &[
    ("0001_init.sql", &[]),
    (
        "0002_volume_music_folder_file.sql",
        &["file", "music_folder", "volume"],
    ),
    (
        "0003_recording_analysis.sql",
        &["analysis", "recording", "recording_file", "version_link"],
    ),
    (
        "0004_library_track_sync.sql",
        &[
            "conflict",
            "library_track",
            "rekordbox_track",
            "relink",
            "sync_base",
        ],
    ),
    (
        "0005_crate_operation_job_setting.sql",
        &[
            "change",
            "crate",
            "crate_entry",
            "job",
            "operation",
            "service_optin",
            "setting",
        ],
    ),
    // Adds `file.file_id`, no tables.
    ("0006_file_id.sql", &[]),
    ("0007_file_stage.sql", &["file_stage"]),
    // Adds `file.online_only` and the music folder's walk counts, no tables.
    ("0008_scan_follow_ups.sql", &[]),
    // Adds `file.partial_hash`, no tables.
    ("0009_partial_hash.sql", &[]),
    // Adds `file.fingerprint_audio_hash`, no tables.
    ("0010_fingerprint_audio_hash.sql", &[]),
    // Adds `rekordbox_track.relink_probable`, no tables.
    ("0011_relink_probable.sql", &[]),
    // Adds `rekordbox_track.relink_audio_hash` and `relink.audio_hash`, no
    // tables.
    ("0012_relink_audio_hash.sql", &[]),
    // Keeps `library_track.source_status` in step with its file; adds the
    // record of tracks the user removed.
    ("0013_library_upkeep.sql", &["library_removal"]),
    // Refuses conflicts on analysis fields, no tables.
    ("0014_analysis_never_conflicts.sql", &[]),
    // The playlists and folders a send has written.
    ("0015_sent_playlist.sql", &["sent_playlist"]),
    // What comparing two files' fingerprints found (derived state).
    ("0016_fingerprint_match.sql", &["fingerprint_match"]),
    // The quality job's measurements; drops `file.cutoff_hz`.
    ("0017_file_quality.sql", &["file_quality"]),
    // `sync_base` ids are never reused. `sqlite_sequence` is SQLite's own
    // table for that (the highest id each AUTOINCREMENT table has held).
    ("0018_sync_base_ids_never_reused.sql", &["sqlite_sequence"]),
    // Which files matching has covered (derived state).
    ("0019_fingerprint_matched.sql", &["fingerprint_matched"]),
    // Adds `file.fingerprint_rev` and its triggers, no tables.
    ("0020_fingerprint_rev.sql", &[]),
    // The stored quality verdict per file (derived state).
    ("0021_file_verdict.sql", &["file_verdict"]),
    // What the grouping job decided, and what the user decided for it.
    (
        "0022_grouping.sql",
        &["grouping_decision", "grouping_file", "grouping_pair"],
    ),
    ("0023_file_roles.sql", &["best_file_due", "better_file"]),
    // The search index (derived state): an FTS5 table, which SQLite backs
    // with shadow tables (`FTS_TABLES`), and the list of tracks due.
    (
        "0024_search_index.sql",
        &[
            "search_due",
            "search_track",
            "search_track_config",
            "search_track_data",
            "search_track_docsize",
            "search_track_idx",
        ],
    ),
];

/// The full-text tables, by exact name: each virtual table with the shadow
/// tables SQLite keeps for it. These are the only tables that may be
/// something other than STRICT, and the only virtual or shadow tables the
/// schema may hold: [`every_phase_1_table_is_strict_so_a_wrongly_typed_value_is_refused`]
/// fails for any other.
pub(super) const FTS_TABLES: &[(&str, &[&str])] = &[(
    "search_track",
    &[
        "search_track_config",
        "search_track_data",
        "search_track_docsize",
        "search_track_idx",
    ],
)];

#[test]
fn each_migration_applies_cleanly_on_top_of_the_ones_before_it_and_adds_exactly_its_tables() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    let mut expected: Vec<String> = vec!["schema_migration".into()];
    for (n, (name, added)) in TABLES_BY_MIGRATION.iter().enumerate() {
        assert_eq!(
            MIGRATIONS[n].name,
            *name,
            "migration {} is not {name}",
            n + 1
        );
        // One more migration per run, as if each release added one.
        assert_eq!(run(&mut conn, &MIGRATIONS[..=n]).unwrap(), 1, "{name}");
        assert_eq!(applied(&conn).unwrap().len(), n + 1);
        expected.extend(added.iter().map(|t| t.to_string()));
        expected.sort();
        assert_eq!(tables(&conn), expected, "after {name}");
        let ok: String = one(&conn, "PRAGMA integrity_check");
        assert_eq!(ok, "ok", "after {name}");
    }
    assert_eq!(
        TABLES_BY_MIGRATION.len(),
        MIGRATIONS.len(),
        "a migration was added; list the tables it adds in TABLES_BY_MIGRATION"
    );
}

#[test]
fn the_phase_1_schema_has_only_the_phase_1_tables() {
    // ROADMAP §2 "When tables arrive": Phase 2 and 3 tables come with the
    // phase that first needs them.
    let (_dir, conn) = db();
    let later = [
        "inbox_batch",
        "inbox_item",
        "wishlist",
        "wish_item",
        "tag",
        "tag_group",
        "genre",
        "genre_alias",
        "playlist",
        "playlist_entry",
        "play_source",
        "play_event",
        "session",
        "embedding",
    ];
    let present = tables(&conn);
    for table in later {
        assert!(!present.iter().any(|t| t == table), "{table} arrived early");
    }
}

/// What is wrong with the tables' types: a table that isn't STRICT, and any
/// virtual or shadow table that isn't one of [`FTS_TABLES`], by exact name
/// and as what it is. Empty when all is well.
fn tables_that_are_not_strict(conn: &Connection) -> Vec<String> {
    let mut problems = Vec::new();
    let mut full_text = 0;
    for table in tables(conn) {
        // SQLite's own tables (`sqlite_sequence`) aren't ours to declare.
        if table == "schema_migration" || table.starts_with("sqlite_") {
            continue;
        }
        let (kind, strict): (String, bool) = conn
            .query_row(
                "SELECT type, strict FROM pragma_table_list WHERE name = ?1",
                [&table],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        match kind.as_str() {
            "virtual" if FTS_TABLES.iter().any(|(t, _)| *t == table) => {
                let sql: String = one(
                    conn,
                    &format!("SELECT sql FROM sqlite_master WHERE name = '{table}'"),
                );
                if sql.contains("USING fts5") {
                    full_text += 1;
                } else {
                    problems.push(format!("{table} is not an FTS5 table"));
                }
            }
            "shadow"
                if FTS_TABLES
                    .iter()
                    .any(|(_, shadows)| shadows.contains(&table.as_str())) =>
            {
                full_text += 1;
            }
            "table" if strict => {}
            "table" => problems.push(format!("{table} is not STRICT")),
            other => problems.push(format!("{table} is a {other} table not in FTS_TABLES")),
        }
    }
    let listed: usize = FTS_TABLES.iter().map(|(_, s)| 1 + s.len()).sum();
    if full_text != listed {
        problems.push(format!("{listed} full-text tables listed, {full_text} found"));
    }
    problems
}

#[test]
fn every_phase_1_table_is_strict_so_a_wrongly_typed_value_is_refused() {
    let (_dir, conn) = db();
    assert_eq!(tables_that_are_not_strict(&conn), Vec::<String>::new());
}

#[test]
fn only_the_listed_full_text_tables_are_excused_from_being_strict() {
    // The excuse is for the search index and its shadow tables by exact
    // name: another full-text table, an ordinary table that isn't STRICT,
    // or a table dressed up with a listed name, still fails.
    let (_dir, conn) = db();
    accepts(&conn, "CREATE VIRTUAL TABLE stray USING fts5(a)");
    accepts(&conn, "CREATE TABLE loose (a)");
    let problems = tables_that_are_not_strict(&conn);
    assert!(problems.iter().any(|p| p.starts_with("stray is a virtual")), "{problems:?}");
    assert!(problems.iter().any(|p| p.starts_with("stray_data is a shadow")), "{problems:?}");
    assert!(problems.contains(&"loose is not STRICT".to_string()), "{problems:?}");
}

#[test]
fn every_foreign_key_lookup_uses_an_index_not_a_table_scan() {
    // Deleting or re-keying a parent row makes SQLite look up its children
    // by the referencing column (e.g. DELETE FROM recording checks
    // analysis, version_link, rekordbox_track…). Without an index that is
    // a full scan of the child table per deleted row: slow at 100k tracks.
    let (_dir, conn) = db();
    let mut checked = 0;
    for table in tables(&conn) {
        let mut stmt = conn
            .prepare("SELECT \"from\", \"table\" FROM pragma_foreign_key_list(?1)")
            .unwrap();
        let keys: Vec<(String, String)> = stmt
            .query_map([&table], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (column, parent) in keys {
            let mut plan = conn
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN SELECT 1 FROM {table} WHERE {column} = ?1"
                ))
                .unwrap();
            let steps: Vec<String> = plan
                .query_map([0], |r| r.get(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(
                steps.iter().all(|s| !s.starts_with("SCAN")),
                "{table}.{column} → {parent} has no index: {steps:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 19, "only {checked} foreign keys found");
}

#[test]
fn a_database_with_data_in_it_takes_the_later_migrations_cleanly() {
    // A real upgrade: rows written at one version, then the rest applied.
    let (dir, conn) = db_at(2);
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');",
    );
    drop(conn);
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    assert_eq!(
        run(&mut conn, MIGRATIONS).unwrap(),
        MIGRATIONS.len() - 2,
        "the later migrations weren't all applied"
    );
    let files: i64 = one(&conn, "SELECT COUNT(*) FROM file");
    assert_eq!(files, 1);
}
