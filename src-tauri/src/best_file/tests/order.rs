//! What holds for the order as a whole, and what the reasons say.

use super::*;

fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![vec![]];
    }
    let mut all = Vec::new();
    for rest in permutations(n - 1) {
        for at in 0..=rest.len() {
            let mut one = rest.clone();
            one.insert(at, n - 1);
            all.push(one);
        }
    }
    all
}

/// Five files that differ at different steps.
fn five_files() -> Vec<FileFacts> {
    vec![
        file(1),
        with(file(2), |f| f.quality.cutoff_hz = Some(17_000)),
        with(file(3), |f| f.tags = full_tags()),
        with(file(4), |f| f.on_disk = false),
        with(file(5), |f| f.quality.verdict = Some(Verdict::Truncated)),
    ]
}

#[test]
fn ranking_a_track_with_no_files_gives_no_best_file() {
    let ranking = rank(&[]);
    assert_eq!(ranking.best(), None);
    assert!(ranking.order.is_empty());
}

#[test]
fn a_track_with_one_file_ranks_it_first_with_only_file_as_the_reason() {
    let ranking = rank(&[file(7)]);
    assert_eq!(ranking.best(), Some(7));
    assert_eq!(ranking.order.len(), 1);
    assert_eq!(ranking.order[0].reason, Reason::OnlyFile);
    assert_eq!(ranking.order[0].reason.step(), None);
}

#[test]
fn the_same_files_in_any_input_order_give_the_same_ranking() {
    let files = five_files();
    let expected = rank(&files);
    assert_eq!(ids(&expected), vec![3, 1, 2, 5, 4]);
    for permutation in permutations(files.len()) {
        let shuffled: Vec<FileFacts> = permutation.iter().map(|i| files[*i].clone()).collect();
        assert_eq!(rank(&shuffled), expected, "{permutation:?}");
    }
}

/// Files over four dimensions: 48 files, all different.
fn pool() -> Vec<FileFacts> {
    let mut pool = Vec::new();
    let mut id = 0;
    for on_disk in [true, false] {
        for verdict in [
            Some(Verdict::Ok),
            None,
            Some(Verdict::LowBitrate),
            Some(Verdict::Truncated),
        ] {
            for codec in [Codec::Flac, Codec::Mp3, Codec::Aac] {
                for tagged in [true, false] {
                    id += 1;
                    pool.push(with(file(id), |f| {
                        f.on_disk = on_disk;
                        f.quality.verdict = verdict;
                        f.format.codec = Some(codec);
                        if tagged {
                            f.tags = full_tags();
                        }
                        // Spread the cutoffs, so the ties are few.
                        f.quality.cutoff_hz = Some(14_000 + (id as u32 % 7) * 1_000);
                    }));
                }
            }
        }
    }
    pool
}

#[test]
fn the_order_is_total_exactly_one_of_two_different_files_is_ahead() {
    let pool = pool();
    for a in &pool {
        for b in &pool {
            let a_ahead = why_ahead(a, b).is_some();
            let b_ahead = why_ahead(b, a).is_some();
            if a.file_id == b.file_id {
                assert!(!a_ahead && !b_ahead);
            } else {
                assert!(a_ahead != b_ahead, "{} and {}", a.file_id, b.file_id);
            }
        }
    }
}

#[test]
fn the_order_is_transitive() {
    let pool = pool();
    let ahead = |a: &FileFacts, b: &FileFacts| why_ahead(a, b).is_some();
    for a in &pool {
        for b in &pool {
            if !ahead(a, b) {
                continue;
            }
            for c in &pool {
                if ahead(b, c) {
                    assert!(
                        ahead(a, c),
                        "{} > {} > {} but not {} > {}",
                        a.file_id,
                        b.file_id,
                        c.file_id,
                        a.file_id,
                        c.file_id
                    );
                }
            }
        }
    }
}

#[test]
fn a_file_is_never_ahead_of_itself() {
    let a = file(1);
    assert_eq!(why_ahead(&a, &a), None);
}

#[test]
fn a_flagged_file_never_outranks_an_ok_or_unchecked_one_whatever_its_other_facts() {
    // The flagged file is the best at every later step; the other is the
    // worst at all of them. Both are on disk.
    for flagged in [Verdict::LowBitrate, Verdict::SuspectTranscode] {
        for other in [Some(Verdict::Ok), None] {
            let strong = with(file(1), |f| {
                f.quality.verdict = Some(flagged);
                f.quality.cutoff_hz = Some(22_000);
                f.format = Format {
                    codec: Some(Codec::Flac),
                    kbps: None,
                };
                f.tags = full_tags();
                f.art = Some(picture(90_000));
                f.rekordbox = RekordboxLink::Trusted;
            });
            let weak = with(file(2), |f| {
                f.quality = Quality {
                    verdict: other,
                    cutoff_hz: None,
                    decode_errors: 40,
                };
                f.plays_in_dj_app = true;
                f.format = Format {
                    codec: Some(Codec::Other),
                    kbps: None,
                };
                f.place.volume_online = false;
                f.place.volume_kind = VolumeKind::Network;
                f.place.folder_role = FolderRole::Inbox;
            });
            let why = why_ahead(&weak, &strong).unwrap();
            assert_eq!(
                why.step(),
                Some(Step::Verdict),
                "{flagged:?} against {other:?}"
            );
            assert_eq!(why_ahead(&strong, &weak), None);
        }
    }
}

#[test]
fn a_file_that_is_gone_is_behind_a_flagged_file_that_is_there() {
    // The one exception to "flagged never outranks ok": a file that isn't
    // there can't be played or linked.
    let gone_but_ok = with(file(1), |f| f.on_disk = false);
    let there_but_flagged = with(file(2), |f| f.quality.verdict = Some(Verdict::LowBitrate));
    let why = why_ahead(&there_but_flagged, &gone_but_ok).unwrap();
    assert_eq!(why, Reason::NotOnDisk);
}

#[test]
fn every_other_rows_reason_names_the_first_step_that_separates_it_from_the_best() {
    let best = with(file(1), |f| {
        f.tags = full_tags();
        f.art = Some(picture(10));
        f.rekordbox = RekordboxLink::Trusted;
    });
    // Differs from the best at art only.
    let no_art = with(best.clone(), |f| {
        f.file_id = 2;
        f.art = None;
    });
    // Differs at tags and everything after.
    let few_tags = with(best.clone(), |f| {
        f.file_id = 3;
        f.tags = TagFields::default();
        f.art = None;
        f.rekordbox = RekordboxLink::Unknown;
    });
    // Differs at the cutoff, and at everything after.
    let low_cutoff = with(few_tags.clone(), |f| {
        f.file_id = 4;
        f.quality.cutoff_hz = Some(16_000);
    });
    // Differs at the verdict first.
    let flagged = with(low_cutoff.clone(), |f| {
        f.file_id = 5;
        f.quality.verdict = Some(Verdict::SuspectTranscode);
    });
    let files = [flagged, few_tags, best, low_cutoff, no_art];
    let ranking = rank(&files);
    assert_eq!(ids(&ranking), vec![1, 2, 3, 4, 5]);
    let steps: Vec<Option<Step>> = ranking.order.iter().map(|r| r.reason.step()).collect();
    assert_eq!(
        steps,
        vec![
            // Best against the second: art.
            Some(Step::Art),
            Some(Step::Art),
            Some(Step::Tags),
            Some(Step::Cutoff),
            Some(Step::Verdict),
        ]
    );
}

#[test]
fn the_best_rows_reason_is_why_it_is_ahead_of_the_second_not_of_the_last() {
    let best = with(file(1), |f| f.art = Some(picture(5)));
    let second = file(2);
    let last = with(file(3), |f| f.quality.verdict = Some(Verdict::Broken));
    let ranking = rank(&[last, second, best]);
    assert_eq!(ids(&ranking), vec![1, 2, 3]);
    // Ahead of the second by art; ahead of the last by the verdict.
    assert_eq!(ranking.order[0].reason.step(), Some(Step::Art));
    assert_eq!(ranking.order[2].reason.step(), Some(Step::Verdict));
}

#[test]
fn why_ahead_agrees_with_the_ranking_for_every_pair() {
    let files = five_files();
    let ranking = rank(&files);
    let best = files
        .iter()
        .find(|f| f.file_id == ranking.best().unwrap())
        .unwrap();
    for row in ranking.order.iter().skip(1) {
        let other = files.iter().find(|f| f.file_id == row.file_id).unwrap();
        assert_eq!(why_ahead(best, other), Some(row.reason.clone()));
    }
}

#[test]
fn a_flac_against_a_320_mp3_is_a_format_reason_with_both_formats() {
    let flac = with(file(1), |f| {
        f.format = Format {
            codec: Some(Codec::Flac),
            kbps: None,
        }
    });
    let mp3 = with(file(2), |_| {});
    let why = why_ahead(&flac, &mp3).unwrap();
    assert_eq!(
        why,
        Reason::BetterFormat {
            best: Format {
                codec: Some(Codec::Flac),
                kbps: None
            },
            other: Format {
                codec: Some(Codec::Mp3),
                kbps: Some(320)
            },
        }
    );
    assert_eq!(
        serde_json::to_value(&why).unwrap(),
        serde_json::json!({
            "code": "betterFormat",
            "best": { "codec": "flac", "kbps": null },
            "other": { "codec": "mp3", "kbps": 320 },
        })
    );
}

#[test]
fn a_trusted_rekordbox_match_is_ahead_of_a_probable_one() {
    let trusted = with(file(2), |f| f.rekordbox = RekordboxLink::Trusted);
    let probable = with(file(1), |f| f.rekordbox = RekordboxLink::Probable);
    assert_eq!(
        why_ahead(&trusted, &probable),
        Some(Reason::KnownToRekordbox {
            best: RekordboxLink::Trusted,
            other: RekordboxLink::Probable,
        })
    );
    assert_eq!(why_ahead(&probable, &trusted), None);
}

#[test]
fn a_probable_rekordbox_match_is_ahead_of_a_file_rekordbox_does_not_know() {
    let probable = with(file(2), |f| f.rekordbox = RekordboxLink::Probable);
    let unknown = with(file(1), |f| f.rekordbox = RekordboxLink::Unknown);
    assert_eq!(
        why_ahead(&probable, &unknown),
        Some(Reason::KnownToRekordbox {
            best: RekordboxLink::Probable,
            other: RekordboxLink::Unknown,
        })
    );
    assert_eq!(why_ahead(&unknown, &probable), None);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "distinct")]
fn two_files_with_the_same_id_are_refused_in_debug_builds() {
    let _ = rank(&[file(1), file(1)]);
}

#[test]
fn reasons_serialize_with_a_camel_case_code_and_camel_case_facts() {
    let cut = Reason::HigherCutoff {
        best_hz: Some(20_000),
        other_hz: None,
    };
    assert_eq!(
        serde_json::to_value(&cut).unwrap(),
        serde_json::json!({ "code": "higherCutoff", "bestHz": 20000, "otherHz": null })
    );
    assert_eq!(
        serde_json::to_value(Reason::OnlyFile).unwrap(),
        serde_json::json!({ "code": "onlyFile" })
    );
    let place = Reason::BetterPlace {
        best: file(1).place,
        other: with(file(2), |f| f.place.volume_online = false).place,
    };
    assert_eq!(
        serde_json::to_value(&place).unwrap()["best"],
        serde_json::json!({
            "volumeOnline": true,
            "folderRole": "scan",
            "volumeKind": "internal",
            "folderOrder": 1,
        })
    );
    assert_eq!(
        serde_json::to_value(Reason::NotOnDisk).unwrap()["code"],
        "notOnDisk"
    );
}
