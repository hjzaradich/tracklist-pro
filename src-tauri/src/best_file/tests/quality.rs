//! Verdict classes, the cutoff's steps, the format lists, tag
//! completeness.

use super::*;

fn flagged(id: i64) -> FileFacts {
    with(file(id), |f| f.quality.verdict = Some(Verdict::LowBitrate))
}

fn with_codec(id: i64, codec: Codec) -> FileFacts {
    with(file(id), |f| {
        f.format = Format {
            codec: Some(codec),
            kbps: None,
        }
    })
}

// ---- Verdict --------------------------------------------------------

#[test]
fn verdicts_rank_ok_then_not_checked_then_flagged_then_truncated_then_broken() {
    let order = [
        Some(Verdict::Ok),
        None,
        Some(Verdict::LowBitrate),
        Some(Verdict::Truncated),
        Some(Verdict::Broken),
    ];
    // Ids run against the order, so only the verdict can put them right.
    let files: Vec<FileFacts> = order
        .iter()
        .enumerate()
        .map(|(i, verdict)| {
            with(file(10 - i as i64), |f| {
                f.quality.verdict = *verdict;
                // Equal cutoffs where counted.
                f.quality.cutoff_hz = Some(20_000);
            })
        })
        .collect();
    let mut reversed = files.clone();
    reversed.reverse();
    assert_eq!(ids(&rank(&reversed)), vec![10, 9, 8, 7, 6]);
}

#[test]
fn low_bitrate_and_suspect_transcode_are_one_tier_the_verdict_does_not_separate_them() {
    let low = with(file(2), |f| f.quality.verdict = Some(Verdict::LowBitrate));
    let transcode = with(file(1), |f| {
        f.quality.verdict = Some(Verdict::SuspectTranscode)
    });
    // Same cutoff: nothing but the id separates them.
    assert_eq!(why_ahead(&transcode, &low), Some(Reason::Tied));
}

#[test]
fn the_cutoff_separates_a_flagged_flac_from_a_flagged_128_kbps_mp3() {
    // A FLAC made from a lossy file, against a real 128 kbps MP3: both
    // flagged, and the header says nothing about quality.
    let flac = with(file(2), |f| {
        f.quality.verdict = Some(Verdict::SuspectTranscode);
        f.quality.cutoff_hz = Some(17_500);
        f.format = Format {
            codec: Some(Codec::Flac),
            kbps: None,
        };
    });
    let mp3 = with(file(1), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate);
        f.quality.cutoff_hz = Some(16_000);
        f.format.kbps = Some(128);
    });
    let why = why_ahead(&flac, &mp3).unwrap();
    assert_eq!(why.step(), Some(Step::Cutoff));
}

#[test]
fn the_measured_cutoff_beats_the_header_bitrate() {
    let measured_high = with(file(2), |f| {
        f.quality.verdict = Some(Verdict::LowBitrate);
        f.quality.cutoff_hz = Some(18_000);
        f.format.kbps = Some(128);
    });
    let header_high = with(file(1), |f| {
        f.quality.verdict = Some(Verdict::SuspectTranscode);
        f.quality.cutoff_hz = Some(15_500);
        f.format.kbps = Some(320);
    });
    let why = why_ahead(&measured_high, &header_high).unwrap();
    assert_eq!(why.step(), Some(Step::Cutoff));
}

// ---- Cutoff ---------------------------------------------------------

fn at_cutoff(id: i64, hz: Option<u32>) -> FileFacts {
    with(file(id), |f| f.quality.cutoff_hz = hz)
}

#[test]
fn every_cutoff_from_19_khz_up_counts_as_full_band() {
    let a = at_cutoff(1, Some(19_000));
    let b = at_cutoff(2, Some(22_050));
    // Tied at the cutoff: only the id is left.
    assert_eq!(why_ahead(&a, &b), Some(Reason::Tied));
}

#[test]
fn a_cutoff_just_under_19_khz_is_behind_one_at_19_khz() {
    let under = at_cutoff(1, Some(18_999));
    let at = at_cutoff(2, Some(19_000));
    assert_eq!(why_ahead(&at, &under).unwrap().step(), Some(Step::Cutoff));
}

#[test]
fn cutoffs_within_one_khz_step_are_the_same_quality() {
    let a = at_cutoff(2, Some(18_000));
    let b = at_cutoff(1, Some(18_999));
    assert_eq!(why_ahead(&b, &a), Some(Reason::Tied));
}

#[test]
fn cutoffs_in_different_khz_steps_are_different_quality() {
    let a = at_cutoff(1, Some(17_999));
    let b = at_cutoff(2, Some(18_000));
    assert_eq!(why_ahead(&b, &a).unwrap().step(), Some(Step::Cutoff));
}

#[test]
fn a_file_with_no_measured_cutoff_is_behind_any_measured_one_of_its_tier() {
    let measured = with(flagged(2), |f| f.quality.cutoff_hz = Some(9_000));
    let unmeasured = with(flagged(1), |f| f.quality.cutoff_hz = None);
    let why = why_ahead(&measured, &unmeasured).unwrap();
    assert_eq!(
        why,
        Reason::HigherCutoff {
            best_hz: Some(9_000),
            other_hz: None
        }
    );
}

#[test]
fn a_cutoff_without_a_verdict_is_not_counted() {
    // No verdict: nothing vouches that the cutoff is of this audio.
    let stale = with(file(2), |f| {
        f.quality = Quality {
            verdict: None,
            cutoff_hz: Some(22_000),
            decode_errors: 0,
        }
    });
    let none = with(file(1), |f| f.quality = Quality::NOT_CHECKED);
    assert_eq!(why_ahead(&none, &stale), Some(Reason::Tied));
}

#[test]
fn the_cutoff_reason_carries_the_measured_values_not_the_steps() {
    let high = at_cutoff(2, Some(20_300));
    let low = at_cutoff(1, Some(16_450));
    assert_eq!(
        why_ahead(&high, &low),
        Some(Reason::HigherCutoff {
            best_hz: Some(20_300),
            other_hz: Some(16_450)
        })
    );
}

#[test]
fn fewer_decode_errors_only_separate_files_whose_cutoffs_tie() {
    let clean_low = with(file(2), |f| f.quality.cutoff_hz = Some(16_000));
    let errors_high = with(file(1), |f| f.quality.decode_errors = 3);
    assert_eq!(
        why_ahead(&errors_high, &clean_low).unwrap().step(),
        Some(Step::Cutoff)
    );
    let clean = with(file(2), |_| {});
    assert_eq!(
        why_ahead(&clean, &errors_high),
        Some(Reason::FewerDecodeErrors { best: 0, other: 3 })
    );
}

#[test]
fn decode_errors_come_from_the_assessments_note() {
    use crate::quality::verdict::{Assessment, Finding};
    let assessment = Assessment {
        verdict: Some(Verdict::Ok),
        findings: vec![Finding::DecodeErrors { count: 6 }],
    };
    let quality = Quality::from_assessment(&assessment, Some(20_000));
    assert_eq!(quality.decode_errors, 6);
    assert_eq!(quality.verdict, Some(Verdict::Ok));
    let flagged = Assessment {
        verdict: Some(Verdict::LowBitrate),
        findings: vec![Finding::LowBitrate { kbps: 128 }],
    };
    assert_eq!(Quality::from_assessment(&flagged, None).decode_errors, 0);
}

// ---- Format ---------------------------------------------------------

/// Files with the codecs, ids against the expected order, ranked.
fn ranked_codecs(codecs: &[Codec]) -> Vec<Codec> {
    let files: Vec<FileFacts> = codecs
        .iter()
        .enumerate()
        .map(|(i, codec)| with_codec(100 - i as i64, *codec))
        .rev()
        .collect();
    let by_id: std::collections::HashMap<i64, Codec> = files
        .iter()
        .map(|f| (f.file_id, f.format.codec.unwrap()))
        .collect();
    ids(&rank(&files)).iter().map(|id| by_id[id]).collect()
}

#[test]
fn lossless_codecs_rank_flac_then_wav_and_aiff_then_alac_then_wavpack_then_ape() {
    let expected = [
        Codec::Flac,
        Codec::Pcm,
        Codec::PcmFloat,
        Codec::Alac,
        Codec::WavPack,
        Codec::Ape,
    ];
    assert_eq!(LOSSLESS_ORDER, expected);
    assert_eq!(ranked_codecs(&expected), expected);
}

#[test]
fn lossy_codecs_rank_mp3_then_aac_then_vorbis_then_opus_then_wma() {
    let expected = [
        Codec::Mp3,
        Codec::Aac,
        Codec::Vorbis,
        Codec::Opus,
        Codec::Wma,
    ];
    assert_eq!(LOSSY_ORDER, expected);
    assert_eq!(ranked_codecs(&expected), expected);
}

#[test]
fn lossless_is_ahead_of_lossy_and_lossy_of_the_codecs_nobody_ranked() {
    let expected = [Codec::Ape, Codec::Wma, Codec::Ac3];
    assert_eq!(ranked_codecs(&expected), expected);
}

#[test]
fn every_codec_the_lists_name_is_in_the_class_it_is_listed_under() {
    // A lossless codec beats every lossy one, and a lossy one beats an
    // unranked one: checked codec by codec, so a codec in the wrong list
    // fails here.
    for lossless in LOSSLESS_ORDER {
        for lossy in LOSSY_ORDER {
            let a = with_codec(2, lossless);
            let b = with_codec(1, lossy);
            let why = why_ahead(&a, &b).unwrap();
            assert_eq!(why.step(), Some(Step::Format), "{lossless:?} {lossy:?}");
        }
    }
    for lossy in LOSSY_ORDER {
        let a = with_codec(2, lossy);
        let b = with_codec(1, Codec::Other);
        assert_eq!(why_ahead(&a, &b).unwrap().step(), Some(Step::Format));
    }
}

#[test]
fn a_file_whose_codec_is_not_known_ranks_with_the_unranked() {
    let unknown = with(file(1), |f| f.format.codec = None);
    let mp3 = with_codec(2, Codec::Mp3);
    assert_eq!(
        why_ahead(&mp3, &unknown).unwrap().step(),
        Some(Step::Format)
    );
    // And ties with a codec nobody ranked.
    let other = with_codec(2, Codec::Speex);
    assert_eq!(why_ahead(&unknown, &other), Some(Reason::Tied));
}

#[test]
fn among_lossy_files_of_one_codec_the_higher_header_bitrate_is_ahead() {
    let high = with(file(2), |f| f.format.kbps = Some(320));
    let low = with(file(1), |f| f.format.kbps = Some(256));
    let why = why_ahead(&high, &low).unwrap();
    assert_eq!(why.step(), Some(Step::Format));
}

#[test]
fn a_lossless_files_header_bitrate_means_nothing() {
    let big = with(file(2), |f| {
        f.format = Format {
            codec: Some(Codec::Flac),
            kbps: Some(1_100),
        }
    });
    let small = with(file(1), |f| {
        f.format = Format {
            codec: Some(Codec::Flac),
            kbps: Some(700),
        }
    });
    assert_eq!(why_ahead(&small, &big), Some(Reason::Tied));
}

// ---- Tags -----------------------------------------------------------

#[test]
fn tag_completeness_counts_title_artist_album_genre_label_and_year() {
    let counted = [
        TagField::Title,
        TagField::Artist,
        TagField::Album,
        TagField::Genre,
        TagField::Label,
        TagField::Year,
    ];
    assert_eq!(COMPLETENESS_FIELDS, counted);
    let none = file(1);
    let all = with(file(2), |f| f.tags = full_tags());
    assert_eq!(
        why_ahead(&all, &none),
        Some(Reason::MoreTags { best: 6, other: 0 })
    );
}

#[test]
fn tag_completeness_does_not_count_comment_composer_remixer_isrc_or_numbers() {
    let others = with(file(2), |f| {
        f.tags = TagFields {
            comment: Some("ripped from a pool".into()),
            composer: Some("A. Fenn".into()),
            remixer: Some("Dub Kestrel".into()),
            isrc: Some("QZABC2100001".into()),
            track_no: Some(3),
            disc_no: Some(1),
            album_artist: Some("Halden Rook".into()),
            ..TagFields::default()
        }
    });
    let none = file(1);
    // Equal at the tags step: only the id is left.
    assert_eq!(why_ahead(&none, &others), Some(Reason::Tied));
}

#[test]
fn a_blank_tag_or_a_year_of_zero_is_not_filled() {
    let blank = with(file(2), |f| {
        f.tags = TagFields {
            title: Some("   ".into()),
            artist: Some(String::new()),
            year: Some(0),
            ..TagFields::default()
        }
    });
    let one = with(file(1), |f| {
        f.tags = TagFields {
            title: Some("Velo".into()),
            ..TagFields::default()
        }
    });
    assert_eq!(
        why_ahead(&one, &blank),
        Some(Reason::MoreTags { best: 1, other: 0 })
    );
}
