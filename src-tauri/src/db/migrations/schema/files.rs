//! 0002: `volume`, `music_folder`, `file` (ROADMAP §0.3, §2).

use rusqlite::Connection;

use super::{accepts, db, one, refuses};
use crate::paths::RelPath;
use crate::volume::{identity, IdentitySignals, VolumeId, VolumeKind};

/// A volume, a music folder on it and one file in that folder: ids 1, 1, 1.
pub(in crate::db::migrations) fn with_a_file() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ Music', 'DJ Music');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');",
    );
    (dir, conn)
}

fn with_a_folder() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ Music', 'DJ Music');",
    );
    (dir, conn)
}

fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[test]
fn a_volume_identity_is_never_a_drive_letter_or_a_path() {
    let (_dir, conn) = db();
    for bad in [
        "E:",
        r"E:\",
        r"E:\Music",
        "",
        "serial=NTFS-1A2B:C4D",
        r"\\nas\music",
        "uuid=1",
    ] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO volume (identity, kind) VALUES ({}, 'external')",
                quoted(bad)
            ),
            "CHECK",
        );
    }
    for good in [
        "serial=NTFS-1A2B3C4D",
        "serial=EXFAT-1A2B3C4D+guid={0a1b-c2d3}",
        "guid={0a1b-c2d3}",
        r"unc=\\nas\music",
    ] {
        accepts(
            &conn,
            &format!(
                "INSERT INTO volume (identity, kind) VALUES ({}, 'external')",
                quoted(good)
            ),
        );
    }
}

#[test]
fn every_identity_the_app_builds_fits_the_volume_table_and_reads_back_through_from_stored() {
    let (_dir, conn) = db();
    let local = |serial, guid| IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial,
        filesystem: "NTFS",
        guid,
    };
    let ids = [
        identity(local(Some(0x1A2B_3C4D), None)).unwrap(),
        identity(local(None, Some("{0A1B-C2D3}"))).unwrap(),
        identity(IdentitySignals {
            kind: VolumeKind::Network,
            unc_share: Some(r"\\NAS\Music"),
            ..local(None, None)
        })
        .unwrap(),
    ];
    for id in ids {
        conn.execute(
            "INSERT INTO volume (identity, kind) VALUES (?1, 'external')",
            [id.as_str()],
        )
        .unwrap();
        let stored: String = conn
            .query_row(
                "SELECT identity FROM volume WHERE id = last_insert_rowid()",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(VolumeId::from_stored(stored).unwrap(), id);
    }
}

#[test]
fn a_volume_is_stored_once_per_identity() {
    let (_dir, conn) = db();
    let sql = "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external')";
    accepts(&conn, sql);
    refuses(&conn, sql, "UNIQUE");
}

#[test]
fn a_volume_is_internal_external_or_network() {
    let (_dir, conn) = db();
    for (n, kind) in ["internal", "external", "network"].iter().enumerate() {
        accepts(
            &conn,
            &format!("INSERT INTO volume (identity, kind) VALUES ('guid={{{n}}}', '{kind}')"),
        );
    }
    for bad in ["usb", "External", ""] {
        refuses(
            &conn,
            &format!("INSERT INTO volume (identity, kind) VALUES ('guid={{9}}', '{bad}')"),
            "CHECK",
        );
    }
}

#[test]
fn a_remembered_volume_guid_is_a_braced_guid() {
    let (_dir, conn) = db();
    refuses(
        &conn,
        "INSERT INTO volume (identity, kind, guid) VALUES ('guid={1}', 'external', 'E:')",
        "CHECK",
    );
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind, guid) VALUES ('guid={1}', 'external', '{0a1b}')",
    );
}

#[test]
fn a_music_folder_must_be_on_a_known_volume() {
    let (_dir, conn) = db();
    refuses(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (99, 'x', 'x')",
        "FOREIGN KEY",
    );
}

#[test]
fn a_volume_with_music_folders_on_it_cannot_be_deleted() {
    let (_dir, conn) = with_a_folder();
    refuses(&conn, "DELETE FROM volume", "FOREIGN KEY");
}

#[test]
fn a_music_folder_is_listed_once_per_volume_and_path() {
    let (_dir, conn) = with_a_folder();
    refuses(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ Music', 'DJ Music')",
        "UNIQUE",
    );
    // The root of the volume is a folder like any other.
    accepts(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, '', '')",
    );
}

#[test]
fn a_music_folder_role_is_scan_or_inbox_and_scan_by_default() {
    let (_dir, conn) = with_a_folder();
    let role: String = one(&conn, "SELECT role FROM music_folder");
    assert_eq!(role, "scan");
    accepts(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key, role) VALUES (1, 'In', 'In', 'inbox')",
    );
    refuses(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key, role) VALUES (1, 'X', 'X', 'source')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key, watch) VALUES (1, 'Y', 'Y', 2)",
        "CHECK",
    );
}

/// Relative paths that `paths::RelPath::parse` refuses, because joined to
/// a mount point they could climb out of it or name a drive.
const ESCAPING_PATHS: &[&str] = &[
    "..",
    "../Windows",
    "Music/../../Windows",
    "Music/./a.mp3",
    "./a.mp3",
    "/Music/a.mp3",
    "Music/",
    "Music//a.mp3",
    r"Music\a.mp3",
    r"D:\Windows",
    "D:",
    "a.mp3:stream",
];

#[test]
fn a_path_that_could_climb_out_of_its_folder_or_name_a_drive_is_refused() {
    let (_dir, conn) = with_a_folder();
    for bad in ESCAPING_PATHS {
        assert!(
            RelPath::parse(bad).is_err(),
            "{bad} parses; update this list"
        );
        let q = quoted(bad);
        refuses(
            &conn,
            &format!(
                "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, {q}, 'ok')"
            ),
            "CHECK",
        );
        refuses(
            &conn,
            &format!(
                "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'ok', {q})"
            ),
            "CHECK",
        );
        refuses(
            &conn,
            &format!("INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, {q}, 'ok')"),
            "CHECK",
        );
        refuses(
            &conn,
            &format!("INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'ok', {q})"),
            "CHECK",
        );
    }
}

#[test]
fn awkward_but_valid_names_are_accepted() {
    // Trailing dots, dots inside names, `#`, `%`, accents and emoji.
    let (_dir, conn) = with_a_folder();
    for good in [
        "Q.X.Z./Album One/a.mp3",
        "..a/b..",
        "Sam Sample Jr./b.mp3.",
        "#hashtag/100% Pure + Live & Loud's.mp3",
        "Música/Déjà Nu 🔥.flac",
    ] {
        assert!(RelPath::parse(good).is_ok(), "{good}");
        let q = quoted(good);
        accepts(
            &conn,
            &format!(
                "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, {q}, {q})"
            ),
        );
    }
}

#[test]
fn a_file_path_is_never_empty() {
    // An empty path would be the music folder itself.
    let (_dir, conn) = with_a_folder();
    refuses(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, '', '')",
        "CHECK",
    );
}

#[test]
fn nfd_and_nfc_spellings_of_a_name_are_two_files_with_one_match_key() {
    // §0.3: NTFS keeps both, so both are stored, each with its own on-disk
    // spelling, and they share the NFC match key.
    let (_dir, conn) = with_a_folder();
    let nfd = RelPath::parse("Cafe\u{301}.mp3").unwrap();
    let nfc = RelPath::parse("Caf\u{e9}.mp3").unwrap();
    for rel in [&nfd, &nfc] {
        conn.execute(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, ?1, ?2)",
            [rel.as_str(), rel.match_key()],
        )
        .unwrap();
    }
    let same_key: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file WHERE rel_path_key = ?1",
            [nfc.match_key()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(same_key, 2);
    // And each reads back to the spelling that opens it.
    let mut stmt = conn
        .prepare("SELECT rel_path FROM file ORDER BY id")
        .unwrap();
    let back: Vec<RelPath> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|s| RelPath::parse(&s.unwrap()).unwrap())
        .collect();
    assert_eq!(back, vec![nfd, nfc]);
}

#[test]
fn a_file_is_stored_once_per_music_folder_and_path() {
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3')",
        "UNIQUE",
    );
}

#[test]
fn a_file_must_be_in_a_known_music_folder_and_its_folder_cannot_be_deleted_under_it() {
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (99, 'b.mp3', 'b.mp3')",
        "FOREIGN KEY",
    );
    refuses(&conn, "DELETE FROM music_folder", "FOREIGN KEY");
}

#[test]
fn a_new_file_is_present_and_not_hidden() {
    let (_dir, conn) = with_a_file();
    let (present, hidden): (i64, Option<String>) = conn
        .query_row("SELECT present, hidden_reason FROM file", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((present, hidden), (1, None));
    refuses(&conn, "UPDATE file SET present = 2", "CHECK");
}

#[test]
fn a_file_is_hidden_only_for_a_known_reason() {
    let (_dir, conn) = with_a_file();
    for reason in ["user", "inbox_cleared"] {
        accepts(
            &conn,
            &format!("UPDATE file SET hidden_reason = '{reason}'"),
        );
    }
    accepts(&conn, "UPDATE file SET hidden_reason = NULL");
    for bad in ["ignored", "extra", ""] {
        refuses(
            &conn,
            &format!("UPDATE file SET hidden_reason = '{bad}'"),
            "CHECK",
        );
    }
}

#[test]
fn a_quality_verdict_is_one_of_the_six_verdicts() {
    // ROADMAP 1.6.
    let (_dir, conn) = with_a_file();
    for verdict in [
        "ok",
        "low_bitrate",
        "suspect_transcode",
        "clipped",
        "truncated",
        "broken",
    ] {
        accepts(
            &conn,
            &format!("UPDATE file SET quality_verdict = '{verdict}'"),
        );
    }
    for bad in ["fake", "low bitrate", "OK"] {
        refuses(
            &conn,
            &format!("UPDATE file SET quality_verdict = '{bad}'"),
            "CHECK",
        );
    }
}

#[test]
fn file_hashes_tags_and_audio_properties_have_sane_shapes() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, "UPDATE file SET blake3 = zeroblob(32)");
    refuses(&conn, "UPDATE file SET blake3 = zeroblob(16)", "CHECK");
    accepts(&conn, r#"UPDATE file SET raw_tags = '{"TIT2": "Title"}'"#);
    refuses(&conn, "UPDATE file SET raw_tags = 'not json'", "CHECK");
    refuses(&conn, "UPDATE file SET raw_tags = '[1, 2]'", "CHECK");
    for bad in [
        "size = -1",
        "bitrate = 0",
        "sample_rate = 0",
        "duration_ms = -1",
        "cutoff_hz = 0",
    ] {
        refuses(&conn, &format!("UPDATE file SET {bad}"), "CHECK");
    }
}

#[test]
fn a_value_of_the_wrong_type_is_refused_not_stored() {
    // STRICT: SQLite would otherwise keep 'big' in an INTEGER column.
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        "UPDATE file SET size = 'big'",
        "cannot store TEXT value in INTEGER column",
    );
    refuses(
        &conn,
        "UPDATE file SET blake3 = 'abc'",
        "cannot store TEXT value in BLOB column",
    );
}

#[test]
fn a_file_is_found_by_its_match_key_through_an_index() {
    // rekordbox Locations are matched on the NFC key (§0.3, §5.3).
    let (_dir, conn) = with_a_file();
    let mut stmt = conn
        .prepare("EXPLAIN QUERY PLAN SELECT id FROM file WHERE rel_path_key = 'a.mp3'")
        .unwrap();
    let plan: Vec<String> = stmt
        .query_map([], |r| r.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        plan.iter().any(|d| d.contains("file_by_rel_path_key")),
        "{plan:?}"
    );
}
