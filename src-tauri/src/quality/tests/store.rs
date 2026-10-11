//! The stored verdict (1bC-5): what the database's strings map to, when a
//! stored verdict is stale, and what reads back. Database only: no audio.

use rusqlite::Connection;

use crate::db::migrations::{run, MIGRATIONS};
use crate::quality::store::{
    any_stale, codec_of, read_verdict_of, settle, settle_files, stored_verdict, stored_verdicts,
    ALL_CODECS, CHUNK, INPUTS, KEY_IGNORES,
};
use crate::quality::verdict::{Assessment, Finding, Verdict};
use crate::quality::RULES_VERSION;
use crate::read::{Codec, Verdict as ReadVerdict};

const HASH_A: &str = "zeroblob(34)";
// A different 34-byte hash: x'01' then 33 zero bytes.
const HASH_B: &str = "x'01000000000000000000000000000000000000000000000000000000000000000000'";

/// A migrated database with one volume and one music folder.
fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    conn.execute_batch(
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
    )
    .unwrap();
    (dir, conn)
}

fn exec(conn: &Connection, sql: &str) {
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("{sql}\n{e}"));
}

/// A file with this codec, bitrate and sample rate (the read stage's
/// columns) and an audio hash. Returns its id.
fn file(conn: &Connection, name: &str, codec: &str, kbps: Option<i64>) -> i64 {
    let kbps = kbps.map_or("NULL".to_string(), |k| k.to_string());
    exec(
        conn,
        &format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, codec, bitrate,
                               sample_rate, audio_hash)
             VALUES (1, '{name}', '{name}', '{codec}', {kbps}, 44100, {HASH_A})"
        ),
    );
    conn.last_insert_rowid()
}

/// A complete measurement of the file's current audio, with a cutoff.
fn measure(conn: &Connection, id: i64, cutoff_hz: i64) {
    exec(
        conn,
        &format!(
            "INSERT OR REPLACE INTO file_quality
                 (file_id, audio_hash, method_version, cutoff_hz, decoded_ms, header_ms, ended)
             VALUES ({id}, {HASH_A}, {v}, {cutoff_hz}, 240000, 240000, 'complete')",
            v = crate::quality::VERSION
        ),
    );
}

fn verdict(conn: &Connection, id: i64) -> Option<Verdict> {
    stored_verdict(conn, id).unwrap().verdict
}

fn settled(conn: &mut Connection) {
    settle(conn).unwrap();
    assert!(!any_stale(conn, None).unwrap(), "something stayed stale");
}

#[test]
fn every_codec_name_the_read_stage_writes_maps_to_its_codec() {
    for codec in ALL_CODECS {
        assert_eq!(codec_of(Some(codec.as_str())), Some(codec), "{codec:?}");
    }
}

#[test]
fn the_list_of_codecs_holds_every_codec_once() {
    // An exhaustive match: a codec added to the enum stops this compiling,
    // and its number then fails the length check below until the list has
    // it.
    fn number(codec: Codec) -> usize {
        match codec {
            Codec::Mp3 => 0,
            Codec::Pcm => 1,
            Codec::PcmFloat => 2,
            Codec::AdpcmMs => 3,
            Codec::AdpcmIma => 4,
            Codec::Alaw => 5,
            Codec::Mulaw => 6,
            Codec::Flac => 7,
            Codec::Aac => 8,
            Codec::Alac => 9,
            Codec::Vorbis => 10,
            Codec::Opus => 11,
            Codec::Speex => 12,
            Codec::Ac3 => 13,
            Codec::Eac3 => 14,
            Codec::Wma => 15,
            Codec::WavPack => 16,
            Codec::Musepack => 17,
            Codec::Ape => 18,
            Codec::Other => 19,
        }
    }
    let numbers: Vec<usize> = ALL_CODECS.iter().map(|c| number(*c)).collect();
    assert_eq!(numbers, (0..20).collect::<Vec<_>>());
}

#[test]
fn a_codec_name_this_build_does_not_know_maps_to_no_codec() {
    for name in [None, Some(""), Some("dts"), Some("MP3"), Some("mp3 ")] {
        assert_eq!(codec_of(name), None, "{name:?}");
    }
}

#[test]
fn only_the_read_stages_own_two_verdicts_are_read_as_its_verdict() {
    assert_eq!(
        read_verdict_of(Some("truncated")),
        Some(ReadVerdict::Truncated)
    );
    assert_eq!(read_verdict_of(Some("broken")), Some(ReadVerdict::Broken));
    for other in [
        None,
        Some("ok"),
        Some("low_bitrate"),
        Some("suspect_transcode"),
        Some("clipped"),
        Some("whatever"),
    ] {
        assert_eq!(read_verdict_of(other), None, "{other:?}");
    }
}

#[test]
fn a_file_with_no_verdict_yet_reads_as_none_never_as_ok() {
    let (_dir, conn) = db();
    let id = file(&conn, "a.mp3", "mp3", Some(320));
    measure(&conn, id, 20_000);
    // Never settled: no row.
    let got = stored_verdict(&conn, id).unwrap();
    assert_eq!(got.verdict, None);
    assert!(got.findings.is_empty());
    // A file that isn't there is the same.
    assert_eq!(verdict(&conn, 999), None);
}

#[test]
fn a_file_with_a_measurement_gets_the_verdict_of_it() {
    let (_dir, mut conn) = db();
    let fine = file(&conn, "fine.mp3", "mp3", Some(320));
    let fake = file(&conn, "fake.mp3", "mp3", Some(320));
    measure(&conn, fine, 20_000);
    measure(&conn, fake, 15_000);
    settled(&mut conn);
    assert_eq!(verdict(&conn, fine), Some(Verdict::Ok));
    let fake = stored_verdict(&conn, fake).unwrap();
    assert_eq!(fake.verdict, Some(Verdict::SuspectTranscode));
    assert_eq!(
        fake.findings,
        [Finding::SuspectTranscodeLossy {
            cutoff_hz: 15_000,
            kbps: 320
        }]
    );
}

#[test]
fn a_file_with_no_measurement_gets_the_verdict_its_headers_and_the_read_stage_give() {
    let (_dir, mut conn) = db();
    let low = file(&conn, "low.mp3", "mp3", Some(128));
    let plain = file(&conn, "plain.mp3", "mp3", Some(320));
    let cut = file(&conn, "cut.mp3", "mp3", Some(320));
    exec(
        &conn,
        &format!("UPDATE file SET quality_verdict = 'truncated' WHERE id = {cut}"),
    );
    settled(&mut conn);
    let low = stored_verdict(&conn, low).unwrap();
    assert_eq!(low.verdict, Some(Verdict::LowBitrate));
    assert_eq!(low.findings, [Finding::LowBitrate { kbps: 128 }]);
    // Nothing flags it and nobody measured it: no verdict, not "ok".
    assert_eq!(verdict(&conn, plain), None);
    assert_eq!(verdict(&conn, cut), Some(Verdict::Truncated));
}

#[test]
fn every_finding_is_stored_and_read_back_whole() {
    let (_dir, mut conn) = db();
    let id = file(&conn, "a.mp3", "mp3", Some(256));
    exec(
        &conn,
        &format!(
            "INSERT INTO file_quality
                 (file_id, audio_hash, method_version, cutoff_hz, decoded_ms, header_ms,
                  decode_errors, ended)
             VALUES ({id}, {HASH_A}, {v}, 15000, 100000, 240000, 3, 'cut_short')",
            v = crate::quality::VERSION
        ),
    );
    exec(
        &conn,
        &format!("UPDATE file SET quality_verdict = 'truncated' WHERE id = {id}"),
    );
    settled(&mut conn);
    let direct = {
        use crate::quality::ledger::stored;
        use crate::quality::verdict::{assess, Facts};
        let row = stored(&conn, id).unwrap().unwrap();
        let hash = row.audio_hash.clone();
        assess(&Facts {
            codec: Some(Codec::Mp3),
            bitrate_kbps: Some(256),
            sample_rate_hz: Some(44_100),
            read: Some(ReadVerdict::Truncated),
            audio_hash: Some(&hash),
            stored: Some(&row),
        })
    };
    assert!(direct.findings.len() >= 3, "{direct:?}");
    assert_eq!(stored_verdict(&conn, id).unwrap(), direct);
}

#[test]
fn a_finding_survives_being_written_and_read_back_for_every_kind() {
    let all = [
        Finding::BrokenUnplayable,
        Finding::BrokenDamaged,
        Finding::BrokenNoAudio,
        Finding::BrokenGaveUp,
        Finding::TruncatedFile,
        Finding::TruncatedStream,
        Finding::TruncatedShort {
            decoded_ms: 1,
            header_ms: 9_000,
        },
        Finding::SuspectTranscodeLossy {
            cutoff_hz: 15_000,
            kbps: 320,
        },
        Finding::SuspectTranscodeLossless { cutoff_hz: 16_000 },
        Finding::LowBitrate { kbps: 128 },
        Finding::PossiblyLowerQuality {
            cutoff_hz: 18_000,
            kbps: 320,
        },
        Finding::CouldBeBetter {
            kbps: 245,
            full_kbps: 318,
        },
        Finding::DecodeErrors { count: 4 },
    ];
    for finding in all {
        let json = serde_json::to_string(&vec![finding.clone()]).unwrap();
        let back: Vec<Finding> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, [finding], "{json}");
    }
}

#[test]
fn settling_never_writes_the_read_stages_verdict_or_any_other_file_column() {
    let (_dir, mut conn) = db();
    let low = file(&conn, "low.mp3", "mp3", Some(128));
    let cut = file(&conn, "cut.mp3", "mp3", Some(320));
    let fake = file(&conn, "fake.mp3", "mp3", Some(320));
    exec(
        &conn,
        &format!("UPDATE file SET quality_verdict = 'truncated' WHERE id = {cut}"),
    );
    measure(&conn, fake, 15_000);
    let snapshot = |conn: &Connection| -> Vec<(i64, Option<String>)> {
        let mut stmt = conn
            .prepare("SELECT id, quality_verdict FROM file ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let whole_table = |conn: &Connection| -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT json_object('r', f.id, 'c', f.codec, 'b', f.bitrate, 's', f.sample_rate,
                                         'v', f.quality_verdict, 'h', hex(f.audio_hash),
                                         'p', f.rel_path, 'd', f.duration_ms, 'm', f.mtime)
                      FROM file f ORDER BY f.id",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let (verdicts_before, table_before) = (snapshot(&conn), whole_table(&conn));
    assert_eq!(settle(&mut conn).unwrap(), 3);
    // A second pass, and a pass over named files: still nothing written.
    settle_files(&mut conn, &[low, cut, fake]).unwrap();
    exec(&conn, "UPDATE file_verdict SET rules_version = 99");
    settle(&mut conn).unwrap();
    assert_eq!(snapshot(&conn), verdicts_before);
    assert_eq!(whole_table(&conn), table_before);
    // The answers are all in the table of their own.
    assert_eq!(verdict(&conn, low), Some(Verdict::LowBitrate));
    assert_eq!(verdict(&conn, fake), Some(Verdict::SuspectTranscode));
}

#[test]
fn this_modules_sources_never_write_the_file_table() {
    // The runtime test above shows the columns stay as they were; this one
    // keeps a later edit from adding a write to `file` in the module that
    // stores verdicts.
    let source = include_str!("../store.rs");
    for line in source.lines() {
        let code = line.split("//").next().unwrap_or("");
        let upper = code.to_ascii_uppercase();
        assert!(
            !upper.contains("UPDATE FILE ")
                && !upper.contains("INSERT INTO FILE ")
                && !upper.contains("INSERT INTO FILE(")
                && !upper.contains("DELETE FROM FILE"),
            "writes the file table: {line}"
        );
    }
}

#[test]
fn a_change_to_any_input_makes_the_stored_verdict_stale_and_settling_gives_the_new_one() {
    // Each case: how to change one input of a settled, measured file, and
    // what it reads as afterwards (the file starts as an `ok` 320 MP3 with
    // a full-band cutoff and one decode error-free pass).
    struct Case {
        name: &'static str,
        change: String,
        then: Option<Verdict>,
    }
    let case = |name, change: &str, then| Case {
        name,
        change: change.to_owned(),
        then,
    };
    let cases = [
        // `file` columns Facts reads.
        case(
            "codec",
            "UPDATE file SET codec = 'flac' WHERE id = 1",
            Some(Verdict::Ok),
        ),
        case(
            "bitrate",
            "UPDATE file SET bitrate = 128 WHERE id = 1",
            Some(Verdict::LowBitrate),
        ),
        case(
            "sample_rate",
            "UPDATE file SET sample_rate = 48000 WHERE id = 1",
            Some(Verdict::Ok),
        ),
        case(
            "the read stage's verdict",
            "UPDATE file SET quality_verdict = 'broken' WHERE id = 1",
            Some(Verdict::Broken),
        ),
        // The audio changed: the measurement is of other audio now, so it
        // is no longer used, and a file with no usable measurement has no
        // verdict.
        case(
            "audio_hash",
            &format!("UPDATE file SET audio_hash = {HASH_B} WHERE id = 1"),
            None,
        ),
        // Every `file_quality` column.
        case(
            "a new row's cutoff",
            "UPDATE file_quality SET cutoff_hz = 15000",
            Some(Verdict::SuspectTranscode),
        ),
        case(
            "cutoff gap",
            "UPDATE file_quality SET cutoff_hz = NULL, cutoff_gap = 'silent'",
            Some(Verdict::Ok),
        ),
        case(
            "decoded_ms",
            "UPDATE file_quality SET decoded_ms = 100000",
            Some(Verdict::Truncated),
        ),
        case(
            "header_ms",
            "UPDATE file_quality SET header_ms = 100000",
            Some(Verdict::Ok),
        ),
        case(
            "decode_errors",
            "UPDATE file_quality SET decode_errors = 2",
            Some(Verdict::Ok),
        ),
        case(
            "error_kinds",
            r#"UPDATE file_quality SET error_kinds = '{"decode": 2}'"#,
            Some(Verdict::Ok),
        ),
        case(
            "ended",
            "UPDATE file_quality SET ended = 'cut_short'",
            Some(Verdict::Truncated),
        ),
        case(
            "failure",
            "UPDATE file_quality SET failure = 'damaged', cutoff_hz = NULL, decoded_ms = NULL,
                                    header_ms = NULL, ended = NULL",
            Some(Verdict::Broken),
        ),
        // A measurement by another method version is not used.
        case(
            "method_version",
            "UPDATE file_quality SET method_version = method_version + 1",
            None,
        ),
        case(
            "the measurement's audio_hash",
            &format!("UPDATE file_quality SET audio_hash = {HASH_B}"),
            None,
        ),
        case(
            "the measurement going away",
            "DELETE FROM file_quality",
            None,
        ),
    ];
    for Case { name, change, then } in cases {
        let (_dir, mut conn) = db();
        let id = file(&conn, "a.mp3", "mp3", Some(320));
        assert_eq!(id, 1);
        measure(&conn, id, 20_000);
        settled(&mut conn);
        assert_eq!(verdict(&conn, id), Some(Verdict::Ok), "{name}: start");
        assert!(!any_stale(&conn, Some(&[id])).unwrap(), "{name}: fresh");

        exec(&conn, &change);
        assert!(any_stale(&conn, Some(&[id])).unwrap(), "{name}: not stale");
        // A stale verdict is not served: it reads as none until settled.
        assert_eq!(verdict(&conn, id), None, "{name}: served stale");
        settle(&mut conn).unwrap();
        assert!(!any_stale(&conn, Some(&[id])).unwrap(), "{name}: still");
        assert_eq!(verdict(&conn, id), then, "{name}: after");
    }
}

#[test]
fn a_change_to_a_column_the_verdict_does_not_read_leaves_it_current() {
    let (_dir, mut conn) = db();
    let id = file(&conn, "a.mp3", "mp3", Some(320));
    measure(&conn, id, 20_000);
    settled(&mut conn);
    for change in [
        "UPDATE file SET mtime = 5, size = 7, duration_ms = 1000, present = 0",
        "UPDATE file SET rel_path = 'b.mp3', rel_path_key = 'b.mp3'",
        "UPDATE file SET raw_tags = '{}'",
        "UPDATE file_quality SET measured_at = '2001-01-01T00:00:00.000Z'",
    ] {
        exec(&conn, change);
        assert!(!any_stale(&conn, None).unwrap(), "{change}");
        assert_eq!(verdict(&conn, id), Some(Verdict::Ok), "{change}");
    }
}

#[test]
fn a_missing_value_and_an_empty_one_are_different_inputs() {
    let (_dir, mut conn) = db();
    let id = file(&conn, "a.mp3", "mp3", Some(320));
    exec(
        &conn,
        &format!("UPDATE file SET codec = NULL WHERE id = {id}"),
    );
    measure(&conn, id, 20_000);
    settled(&mut conn);
    exec(
        &conn,
        &format!("UPDATE file SET codec = '' WHERE id = {id}"),
    );
    assert!(any_stale(&conn, None).unwrap(), "NULL became ''");
    settled(&mut conn);
    exec(&conn, "UPDATE file_quality SET error_kinds = '{}'");
    assert!(any_stale(&conn, None).unwrap(), "NULL became {{}}");
    settled(&mut conn);
    // And an empty audio hash is not a missing one.
    exec(
        &conn,
        &format!("UPDATE file SET audio_hash = NULL WHERE id = {id}"),
    );
    settled(&mut conn);
    exec(
        &conn,
        &format!("UPDATE file SET audio_hash = x'' WHERE id = {id}"),
    );
    assert!(any_stale(&conn, None).unwrap(), "NULL became an empty hash");
}

#[test]
fn the_key_covers_every_file_quality_column_except_the_ones_it_names_as_ignored() {
    let (_dir, conn) = db();
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_xinfo('file_quality')")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(names.len() >= 11, "{names:?}");
    for name in &names {
        let used = INPUTS.contains(&format!("q.{name})")) || INPUTS.contains(&format!("q.{name},"));
        let ignored = KEY_IGNORES.contains(&name.as_str());
        assert!(
            used != ignored,
            "file_quality.{name}: in the key = {used}, ignored = {ignored}; a column is \
             either in the key or on the ignore list, never both and never neither"
        );
    }
    for ignored in KEY_IGNORES {
        assert!(
            names.iter().any(|n| n == ignored),
            "{ignored} is not a column"
        );
    }
}

#[test]
fn the_key_covers_every_file_column_the_facts_are_built_from() {
    for column in [
        "audio_hash",
        "codec",
        "bitrate",
        "sample_rate",
        "quality_verdict",
    ] {
        assert!(
            INPUTS.contains(&format!("f.{column}")),
            "file.{column} is a Facts source and isn't in the key"
        );
    }
}

#[test]
fn a_new_rules_version_makes_every_verdict_stale_and_they_are_recomputed_from_what_is_stored() {
    let (_dir, mut conn) = db();
    let low = file(&conn, "low.mp3", "mp3", Some(128));
    let fake = file(&conn, "fake.mp3", "mp3", Some(320));
    measure(&conn, fake, 15_000);
    settled(&mut conn);
    // As if the rules were older: another version number, and what they
    // concluded then.
    exec(
        &conn,
        "UPDATE file_verdict SET rules_version = rules_version + 1,
                                 verdict = 'ok', findings = '[]'",
    );
    assert!(any_stale(&conn, None).unwrap());
    assert_eq!(verdict(&conn, low), None);
    assert_eq!(verdict(&conn, fake), None);

    settled(&mut conn);
    assert_eq!(verdict(&conn, low), Some(Verdict::LowBitrate));
    assert_eq!(verdict(&conn, fake), Some(Verdict::SuspectTranscode));
    let version: i64 = conn
        .query_row("SELECT MIN(rules_version) FROM file_verdict", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(version, i64::from(RULES_VERSION));
}

#[test]
fn settling_says_how_many_verdicts_came_out_different() {
    let (_dir, mut conn) = db();
    let low = file(&conn, "low.mp3", "mp3", Some(128));
    let other = file(&conn, "ok.mp3", "mp3", Some(320));
    let _blank = file(&conn, "blank.mp3", "mp3", None);
    measure(&conn, other, 20_000);
    // Two get a first verdict; the third has nothing to say.
    assert_eq!(settle(&mut conn).unwrap(), 2);
    // Nothing is stale.
    assert_eq!(settle(&mut conn).unwrap(), 0);
    // An input moves and the verdict with it.
    exec(
        &conn,
        &format!("UPDATE file SET bitrate = 320 WHERE id = {low}"),
    );
    assert_eq!(settle(&mut conn).unwrap(), 1);
    assert_eq!(verdict(&conn, low), None);
    // An input moves and the verdict stays: stored again, not counted.
    exec(
        &conn,
        &format!("UPDATE file SET sample_rate = 48000 WHERE id = {other}"),
    );
    assert_eq!(settle_files(&mut conn, &[other]).unwrap(), 0);
    assert_eq!(verdict(&conn, other), Some(Verdict::Ok));
}

#[test]
fn settling_named_files_leaves_the_others_stale() {
    let (_dir, mut conn) = db();
    let a = file(&conn, "a.mp3", "mp3", Some(128));
    let b = file(&conn, "b.mp3", "mp3", Some(128));
    settle_files(&mut conn, &[a]).unwrap();
    assert_eq!(verdict(&conn, a), Some(Verdict::LowBitrate));
    assert_eq!(verdict(&conn, b), None);
    assert!(any_stale(&conn, Some(&[b])).unwrap());
    assert!(!any_stale(&conn, Some(&[a])).unwrap());
}

#[test]
fn many_files_are_read_at_once_across_more_than_one_chunk() {
    let (_dir, mut conn) = db();
    let count = CHUNK * 2 + 7;
    exec(&conn, "BEGIN");
    for n in 0..count {
        // Every third file is low bitrate; the rest are plain (no verdict).
        let kbps = if n % 3 == 0 { 128 } else { 320 };
        file(&conn, &format!("{n}.mp3"), "mp3", Some(kbps));
    }
    exec(&conn, "COMMIT");
    settled(&mut conn);
    let mut ids: Vec<i64> = (1..=count as i64).collect();
    // An id that isn't a file is in the answer too, as none.
    ids.push(999_999);
    let got = stored_verdicts(&conn, &ids).unwrap();
    assert_eq!(got.len(), ids.len());
    for (n, id) in (1..=count as i64).enumerate() {
        let expected = (n % 3 == 0).then_some(Verdict::LowBitrate);
        assert_eq!(got[&id].verdict, expected, "file {id}");
    }
    assert_eq!(got[&999_999].verdict, None);
}

#[test]
fn a_stored_row_this_build_cannot_read_reads_as_none() {
    let (_dir, mut conn) = db();
    let id = file(&conn, "a.mp3", "mp3", Some(128));
    settled(&mut conn);
    assert_eq!(verdict(&conn, id), Some(Verdict::LowBitrate));
    // A finding from a later build, current in every other way.
    exec(
        &conn,
        r#"UPDATE file_verdict SET findings = '[{"code": "fromALaterBuild", "x": 1}]'"#,
    );
    let got = stored_verdict(&conn, id).unwrap();
    assert_eq!(
        got,
        Assessment {
            verdict: None,
            findings: Vec::new()
        }
    );
    // Settling doesn't touch a row that looks current…
    settle(&mut conn).unwrap();
    assert_eq!(verdict(&conn, id), None);
    // …and a new rules version replaces it.
    exec(
        &conn,
        "UPDATE file_verdict SET rules_version = rules_version + 1",
    );
    settled(&mut conn);
    assert_eq!(verdict(&conn, id), Some(Verdict::LowBitrate));
}

#[test]
fn a_verdict_is_deleted_with_its_file() {
    let (_dir, mut conn) = db();
    let id = file(&conn, "a.mp3", "mp3", Some(128));
    settled(&mut conn);
    exec(&conn, &format!("DELETE FROM file WHERE id = {id}"));
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_verdict", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

/// Pins what [`assess`] concludes over a grid of inputs, next to the rules
/// version. If this fails, the rules or thresholds changed: bump
/// `RULES_VERSION` (so every stored verdict is recomputed) and put the new
/// version and digest here. Changing the pin without bumping the version
/// is what the test is there to catch.
#[test]
fn a_change_to_the_rules_comes_with_a_new_rules_version() {
    use crate::quality::ledger::Stored;
    use crate::quality::verdict::{assess, Facts};
    use sha2::{Digest, Sha256};

    const PINNED_VERSION: u8 = 1;
    const PINNED_DIGEST: &str = "b65340d386ee0fb3abfe76e68185984d197751970ff7a403ae679939d8415498";

    let hash = vec![7u8; 34];
    let measured =
        |cutoff: Option<i64>, decoded: i64, header: Option<i64>, errors: i64, ended: &str| Stored {
            audio_hash: hash.clone(),
            method_version: i64::from(crate::quality::VERSION),
            cutoff_hz: cutoff,
            cutoff_gap: cutoff.is_none().then(|| "silent".to_owned()),
            decoded_ms: Some(decoded),
            header_ms: header,
            decode_errors: errors,
            error_kinds: None,
            ended: Some(ended.to_owned()),
            failure: None,
        };
    let failed = |why: &str| Stored {
        audio_hash: hash.clone(),
        method_version: i64::from(crate::quality::VERSION),
        cutoff_hz: None,
        cutoff_gap: None,
        decoded_ms: None,
        header_ms: None,
        decode_errors: 0,
        error_kinds: None,
        ended: None,
        failure: Some(why.to_owned()),
    };

    let mut digest = Sha256::new();
    let mut feed = |facts: &Facts<'_>| {
        let out = assess(facts);
        digest.update(serde_json::to_vec(&out).unwrap());
        digest.update(b"\n");
    };
    let reads = [
        None,
        Some(ReadVerdict::Truncated),
        Some(ReadVerdict::Broken),
    ];
    // Codec × bitrate × cutoff × sample rate × read-stage verdict.
    for codec in ALL_CODECS.iter().map(|c| Some(*c)).chain([None]) {
        for kbps in [
            None,
            Some(1),
            Some(64),
            Some(128),
            Some(189),
            Some(190),
            Some(191),
            Some(192),
            Some(239),
            Some(240),
            Some(253),
            Some(254),
            Some(255),
            Some(256),
            Some(317),
            Some(318),
            Some(320),
            Some(900),
        ] {
            for cutoff in [
                None,
                Some(1_000),
                Some(16_999),
                Some(17_000),
                Some(18_999),
                Some(19_000),
                Some(19_499),
                Some(19_500),
                Some(20_000),
                Some(22_050),
            ] {
                let row = measured(cutoff, 240_000, Some(240_000), 0, "complete");
                for rate in [None, Some(32_000), Some(44_100), Some(48_000)] {
                    for read in reads {
                        feed(&Facts {
                            codec,
                            bitrate_kbps: kbps,
                            sample_rate_hz: rate,
                            read,
                            audio_hash: Some(&hash),
                            stored: Some(&row),
                        });
                    }
                }
            }
        }
    }
    // Measurements of one MP3 and one FLAC: lengths, endings, errors,
    // failures, a row that doesn't apply.
    let mut rows = vec![];
    for (decoded, header) in [
        (240_000, Some(240_000)),
        (237_999, Some(240_000)),
        (238_000, Some(240_000)),
        (100_000, Some(240_000)),
        (0, Some(1_000)),
        (1_000, Some(0)),
        (240_000, None),
    ] {
        for errors in [0, 1, 9] {
            for ended in ["complete", "cut_short", "gave_up"] {
                rows.push(measured(Some(20_000), decoded, header, errors, ended));
            }
        }
    }
    for why in [
        "unsupported_format",
        "unsupported_codec",
        "no_audio",
        "damaged",
        "decoder_crashed",
    ] {
        rows.push(failed(why));
    }
    for codec in [Codec::Mp3, Codec::Flac] {
        for row in &rows {
            for hash_now in [Some(&hash[..]), None] {
                feed(&Facts {
                    codec: Some(codec),
                    bitrate_kbps: Some(320),
                    sample_rate_hz: Some(44_100),
                    read: None,
                    audio_hash: hash_now,
                    stored: Some(row),
                });
            }
        }
    }
    let got: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    assert_eq!(
        RULES_VERSION, PINNED_VERSION,
        "RULES_VERSION changed: put the new version and the digest below in this test"
    );
    assert_eq!(
        got, PINNED_DIGEST,
        "the rules or thresholds of `assess` changed: bump RULES_VERSION in \
         quality::store and pin the new version and this digest here"
    );
}
