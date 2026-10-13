//! The queue's cost and its keeping to its indexes and its locale file.

use super::super::held::{components_of, held_pairs_sql, HeldPair};
use super::super::queue::{better_from, quality_from, relink_from, version_doubt_from, DOUBT_PAIR};
use super::*;
use crate::best_file::{Format, Place, Reason as Rank, RekordboxLink};
use crate::grouping::plan::State;
use crate::quality::verdict::Verdict;
use crate::read::Codec;

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `sql` of the index `name`, whitespace squashed.
fn index_sql(lib: &Lib, name: &str) -> String {
    let name = name.to_owned();
    let sql: String = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [name],
                |r| r.get(0),
            )
        })
        .unwrap();
    squash(&sql)
}

#[test]
fn each_index_of_the_migration_has_the_where_the_read_model_uses() {
    let lib = Lib::new();
    for (index, predicate) in [
        ("grouping_pair_pending", PENDING_PAIRS),
        ("file_verdict_flagged", FLAGGED),
        ("rekordbox_track_probable", PROBABLE_RELINK),
    ] {
        let sql = index_sql(&lib, index);
        assert!(
            sql.ends_with(&format!("WHERE {}", squash(predicate))),
            "{index}: {sql}"
        );
    }
}

#[test]
fn the_shared_where_texts_name_exactly_the_states_and_verdicts_they_stand_for() {
    for state in [State::Open, State::SetDisagrees, State::BothInLibrary] {
        assert!(PENDING_PAIRS.contains(&format!("'{}'", state.as_str())));
    }
    for state in State::ALL {
        let pending = [State::Open, State::SetDisagrees, State::BothInLibrary].contains(&state);
        assert_eq!(
            PENDING_PAIRS.contains(&format!("'{}'", state.as_str())),
            pending,
            "{state:?}"
        );
    }
    for verdict in [
        Verdict::Ok,
        Verdict::LowBitrate,
        Verdict::SuspectTranscode,
        Verdict::Truncated,
        Verdict::Broken,
    ] {
        assert_eq!(
            FLAGGED.contains(&format!("'{}'", verdict.as_str())),
            verdict != Verdict::Ok,
            "{verdict:?}"
        );
    }
}

fn pair(a: i64, b: i64, both: bool) -> HeldPair {
    HeldPair {
        a,
        b,
        both_in_library: both,
        a_in_library: a == 1 || both,
        b_in_library: both,
    }
}

#[test]
fn the_id_of_a_held_set_does_not_change_when_its_pairs_are_read_in_another_order() {
    let pairs = vec![
        pair(7, 3, false),
        pair(3, 9, false),
        pair(1, 7, false),
        pair(20, 21, true),
        pair(21, 22, true),
    ];
    let expected = components_of(&pairs);
    assert_eq!(expected.len(), 2);
    assert_eq!(expected[0].anchor, 1);
    assert_eq!(expected[0].tracks, [1, 3, 7, 9]);
    assert!(!expected[0].same_recording);
    assert_eq!(expected[1].anchor, 20);
    assert!(expected[1].same_recording);

    let mut reversed = pairs.clone();
    reversed.reverse();
    assert_eq!(components_of(&reversed), expected);
    for turn in 1..pairs.len() {
        let mut rotated = pairs.clone();
        rotated.rotate_left(turn);
        assert_eq!(components_of(&rotated), expected, "rotated by {turn}");
    }
    // A pair written the other way round is the same pair.
    let flipped: Vec<_> = pairs
        .iter()
        .map(|p| HeldPair {
            a: p.b,
            b: p.a,
            a_in_library: p.b_in_library,
            b_in_library: p.a_in_library,
            ..*p
        })
        .collect();
    assert_eq!(components_of(&flipped), expected);
}

#[test]
fn a_set_with_no_track_in_the_library_is_dropped() {
    let pairs = [HeldPair {
        a: 5,
        b: 6,
        both_in_library: false,
        a_in_library: false,
        b_in_library: false,
    }];
    assert!(components_of(&pairs).is_empty());
}

// ---- reasons and the locale file ------------------------------------------

fn locale_has(key: &str) -> bool {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../../src/locales/en/review.json")).unwrap();
    let path = key.strip_prefix("review:").expect("a review key");
    let mut at = &json;
    for part in path.split('.') {
        match at.get(part) {
            Some(next) => at = next,
            None => return false,
        }
    }
    at.is_string()
}

#[test]
fn every_reason_code_the_read_model_can_send_has_text_in_the_locale_file() {
    use crate::review::reasons::{from_grouping, from_rank, from_relink};
    let format = |codec, kbps| Format {
        codec: Some(codec),
        kbps,
    };
    let place = Place {
        volume_online: true,
        folder_role: crate::best_file::FolderRole::Scan,
        volume_kind: crate::best_file::VolumeKind::Internal,
        folder_order: 1,
    };
    let ranks = [
        Rank::OnlyFile,
        Rank::NotOnDisk,
        Rank::BetterVerdict {
            best: None,
            other: None,
        },
        Rank::Playable,
        Rank::HigherCutoff {
            best_hz: Some(20_000),
            other_hz: Some(16_000),
        },
        Rank::HigherCutoff {
            best_hz: Some(20_000),
            other_hz: None,
        },
        Rank::FewerDecodeErrors { best: 0, other: 3 },
        Rank::BetterFormat {
            best: format(Codec::Flac, None),
            other: format(Codec::Mp3, Some(320)),
        },
        Rank::BetterFormat {
            best: format(Codec::Mp3, Some(320)),
            other: format(Codec::Mp3, Some(128)),
        },
        Rank::MoreTags { best: 3, other: 1 },
        Rank::HasArt,
        Rank::KnownToRekordbox {
            best: RekordboxLink::Trusted,
            other: RekordboxLink::Unknown,
        },
        Rank::BetterPlace {
            best: place,
            other: place,
        },
        Rank::Tied,
    ];
    let mut keys: Vec<String> = ranks
        .iter()
        .map(|r| from_rank(r).key().to_owned())
        .collect();
    for method in [
        "filename_only",
        "unique_duration",
        "fingerprint",
        "filename_duration",
        "user",
        "gig_stick",
    ] {
        keys.push(from_relink(method).key().to_owned());
    }
    // Every code of grouping::rules::Reason::code().
    let codes = [
        "identicalAudio",
        "sameAudio",
        "versionVeto",
        "sameAudioNamesDisagree",
        "audioInsideOther",
        "audioInsideOtherNamesDisagree",
        "sameTagsPartialAudio",
        "partialAudioNamesUnclear",
        "namesSayCut",
        "namesSayRework",
        "mashupContains",
        "labelWordsDiffer",
        "fingerprintVeto",
        "noFingerprint",
        "notCompared",
        "sameTitleDifferentArtists",
        "mashupPart",
        "namesUnclear",
        "titlesDiffer",
        "namesDiffer",
        "artistsDiffer",
        "creditedToRemixer",
    ];
    for code in codes {
        let stored = format!("[{{\"code\":\"{code}\",\"foundA\":0.9,\"onlyA\":[\"clean\"]}}]");
        let made = from_grouping(&stored);
        assert_eq!(made.len(), 1, "{code}");
        // Lists and numbers are not sent; the code is the reason.
        assert!(made[0].params().is_empty());
        keys.push(made[0].key().to_owned());
    }
    let missing: Vec<_> = keys.iter().filter(|k| !locale_has(k)).collect();
    assert!(missing.is_empty(), "no text for {missing:?}");
}

#[test]
fn the_grouping_reasons_shown_are_the_first_three_in_the_stored_order() {
    let stored = r#"[{"code":"sameAudio"},{"code":"namesDiffer"},{"code":"artistsDiffer"},{"code":"titlesDiffer"}]"#;
    let keys: Vec<_> = crate::review::reasons::from_grouping(stored)
        .iter()
        .map(|r| r.key().to_owned())
        .collect();
    assert_eq!(
        keys,
        [
            "review:doubt.sameAudio",
            "review:doubt.namesDiffer",
            "review:doubt.artistsDiffer"
        ]
    );
    assert!(crate::review::reasons::from_grouping("not json").is_empty());
    assert!(crate::review::reasons::from_grouping(r#"[{"code":"a b"}]"#).is_empty());
}

// ---- a library of 100,000 files -----------------------------------------------

fn plan_of(lib: &Lib, sql: &str) -> String {
    let sql = format!("EXPLAIN QUERY PLAN {sql}");
    lib.writer
        .call(move |c| {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(3))?;
            let details: rusqlite::Result<Vec<String>> = rows.collect();
            Ok(details?.join("\n"))
        })
        .unwrap()
}

/// 100,000 tracks with one file each, built by SQL: the first 50,000 are in
/// the Library. See the assertions for what waits.
fn seed_big(lib: &Lib) {
    let n = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)";
    lib.exec(&format!(
        "{n} INSERT INTO recording (id) SELECT i FROM n;
         {n} INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, present,
                               sniffed_format, codec, bitrate)
             SELECT i, 1, 'f' || i || '.mp3', 'f' || i || '.mp3', 1, 'mp3', 'mp3', 320 FROM n;
         {n} INSERT INTO recording_file (recording_id, file_id) SELECT i, i FROM n;
         {n} INSERT INTO library_track (recording_id, kind, linked_file_id)
             SELECT i, 'linked', i FROM n WHERE i <= 50000;
         -- 20,000 doubts that involve a Library track.
         {n} INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
             SELECT i, i + 60000, 'review', '[]', 'open' FROM n WHERE i <= 20000;
         -- 5,000 doubts between tracks that are not in the Library.
         {n} INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
             SELECT i + 80000, i + 85000, 'review', '[]', 'open' FROM n WHERE i <= 5000;
         -- 10,000 Library tracks that are the same recording as another.
         {n} INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
             SELECT i + 20000, i + 30000, 'duplicate', '[]', 'both_in_library'
             FROM n WHERE i <= 10000;
         -- 100 held sets of two.
         {n} INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
             SELECT i + 40000, i + 90000, 'duplicate', '[]', 'set_disagrees'
             FROM n WHERE i <= 100;
         -- Everything else the grouping stored: pairs that wait for nobody.
         {n} INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
             SELECT i + 50000, i + 50001, 'duplicate', '[]', 'merged'
             FROM n WHERE i <= 9000 AND i % 2 = 1;
         -- 500 flagged linked files with a current verdict, and 10,000 ok ones.
         {n} INSERT INTO file_verdict (file_id, rules_version, inputs, verdict, findings)
             SELECT f.id, {rules}, {inputs}, CASE WHEN f.id <= 45500 THEN 'broken' ELSE 'ok' END, '[]'
             FROM n JOIN file f ON f.id = n.i LEFT JOIN file_quality q ON q.file_id = f.id
             WHERE f.id > 45000 AND f.id <= 55000;
         -- 300 probable and 1,000 trusted rekordbox matches.
         {n} INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             SELECT json_object('TrackID', i, 'Name', 'Old', 'Artist', 'Someone',
                                'Location', 'file://localhost/E:/Old/' || i || '.mp3'),
                    'E:/Old/' || i || '.mp3', '2026-09-30T10:00:00.000Z',
                    CASE WHEN i <= 300 THEN i + 91000 ELSE i + 91700 END, 'filename_only',
                    CASE WHEN i <= 300 THEN 1 ELSE 0 END
             FROM n WHERE i <= 1300;",
        rules = crate::quality::RULES_VERSION,
        inputs = crate::quality::store::INPUTS,
    ));
}

#[test]
fn a_library_of_100000_files_is_counted_and_paged_from_the_few_rows_that_wait() {
    let lib = Lib::new();
    seed_big(&lib);

    let timed = |what: &str, run: &dyn Fn()| {
        let start = std::time::Instant::now();
        run();
        // Printed, not asserted: read it with `--nocapture`. A debug build.
        eprintln!("100,000 files, {what}: {:?}", start.elapsed());
    };
    timed("counts", &|| {
        lib.counts();
    });
    timed("first page of Needs a closer look", &|| {
        lib.page(ReviewGroup::NeedsCloserLook, 0, 100);
    });
    timed("a page of held sets", &|| {
        lib.page(ReviewGroup::NeedsCloserLook, 20_000, 100);
    });
    timed("opening a held set", &|| {
        let held = lib.compare(id(ReviewKind::HeldSet, 40_001, None));
        assert_eq!(held.unwrap().rows.len(), 2);
    });
    timed("opening a same-recording item", &|| {
        let same = lib.compare(id(ReviewKind::SameRecording, 20_001, None));
        assert_eq!(same.unwrap().rows.len(), 2);
    });
    timed("opening a version doubt", &|| {
        let doubt = lib.compare(id(ReviewKind::VersionDoubt, 1, Some(60_001)));
        assert_eq!(doubt.unwrap().rows.len(), 2);
    });

    let counts = lib.counts();
    // 20,000 doubts + 100 held sets + 10,000 same-recording pairs + 300 relinks.
    assert_eq!(counts.needs_closer_look, 30_400);
    assert_eq!(counts.clear_best_file, 0);
    // 500 flagged files; the 'ok' ones and the other 4,500 don't count.
    assert_eq!(counts.quality_problems, 500);
    assert_eq!(counts.total, 30_900);

    // A page deep into the first group, and the lists' totals.
    let page = lib.page(ReviewGroup::NeedsCloserLook, 19_990, 20);
    assert_eq!(page.total, 30_400);
    let kinds: Vec<_> = page.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == ReviewKind::VersionDoubt)
            .count(),
        10
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == ReviewKind::HeldSet).count(),
        10
    );
    assert_eq!(lib.page(ReviewGroup::QualityProblems, 0, 5).total, 500);

    // The queries stand on the new indexes, not on a walk of the tables.
    let plan = plan_of(
        &lib,
        &format!(
            "SELECT {DOUBT_PAIR} {} GROUP BY 1, 2 ORDER BY 1, 2 LIMIT 100 OFFSET 0",
            version_doubt_from("")
        ),
    );
    assert!(plan.contains("grouping_pair_pending"), "{plan}");
    let plan = plan_of(&lib, &held_pairs_sql());
    assert!(plan.contains("grouping_pair_pending"), "{plan}");
    let plan = plan_of(
        &lib,
        &format!(
            "SELECT lt.recording_id {} ORDER BY 1 LIMIT 100",
            quality_from("")
        ),
    );
    assert!(plan.contains("file_verdict_flagged"), "{plan}");
    let plan = plan_of(&lib, &format!("SELECT count(*) {}", relink_from("")));
    assert!(plan.contains("rekordbox_track_probable"), "{plan}");
    // The better-file kinds walk the small `better_file` table (one row per
    // track with a better copy) and look each track up; never the whole
    // Library.
    let plan = plan_of(
        &lib,
        &format!("SELECT lt.recording_id {}", better_from(true, "")),
    );
    assert!(
        plan.contains("SCAN bf") && !plan.contains("SCAN lt"),
        "{plan}"
    );
}
