//! Behavior tests for the verdicts (ROADMAP 1.6; 1bB-6). Pure: facts in,
//! verdict and findings out. Every threshold has a test on both sides of
//! its line, written with the number, not the constant, so moving a line
//! fails one of them.

use crate::quality::verdict::{assess, Assessment, Facts, Finding, Verdict};
use crate::quality::{Ended, Failure, Gap, Stored, VERSION};
use crate::read::{Codec, Verdict as ReadVerdict};

const HASH: [u8; 34] = [7; 34];

/// A measured row of the audio `HASH`: complete, clean, 4 minutes long,
/// with a cutoff of 20 kHz.
fn row() -> Stored {
    Stored {
        audio_hash: HASH.to_vec(),
        method_version: i64::from(VERSION),
        cutoff_hz: Some(20_000),
        cutoff_gap: None,
        decoded_ms: Some(240_000),
        header_ms: Some(240_000),
        decode_errors: 0,
        error_kinds: None,
        ended: Some(Ended::Complete.as_str().to_owned()),
        failure: None,
    }
}

fn with_cutoff(hz: i64) -> Stored {
    Stored {
        cutoff_hz: Some(hz),
        ..row()
    }
}

fn with_lengths(decoded_ms: i64, header_ms: i64) -> Stored {
    Stored {
        decoded_ms: Some(decoded_ms),
        header_ms: Some(header_ms),
        ..row()
    }
}

fn failed(why: Failure) -> Stored {
    Stored {
        cutoff_hz: None,
        decoded_ms: None,
        header_ms: None,
        ended: None,
        failure: Some(why.as_str().to_owned()),
        ..row()
    }
}

fn ended(how: Ended) -> Stored {
    Stored {
        ended: Some(how.as_str().to_owned()),
        ..row()
    }
}

/// A file of `codec` at `kbps` and 44.1 kHz, with this row, its audio
/// hash known.
fn assess_row(codec: Codec, kbps: i64, stored: &Stored) -> Assessment {
    assess(&Facts {
        codec: Some(codec),
        bitrate_kbps: Some(kbps),
        sample_rate_hz: Some(44_100),
        read: None,
        audio_hash: Some(&HASH[..]),
        stored: Some(stored),
    })
}

fn mp3(kbps: i64, stored: &Stored) -> Assessment {
    assess_row(Codec::Mp3, kbps, stored)
}

fn lossless(codec: Codec, stored: &Stored) -> Assessment {
    assess_row(codec, 900, stored)
}

/// Facts with no measurement at all.
fn unmeasured(codec: Option<Codec>, kbps: Option<i64>, read: Option<ReadVerdict>) -> Assessment {
    assess(&Facts {
        codec,
        bitrate_kbps: kbps,
        sample_rate_hz: Some(44_100),
        read,
        audio_hash: Some(&HASH[..]),
        stored: None,
    })
}

fn flagged(a: &Assessment) -> Option<Verdict> {
    a.verdict.filter(|v| *v != Verdict::Ok)
}

fn is_ok_without_notes(a: &Assessment) -> bool {
    a.verdict == Some(Verdict::Ok) && a.findings.is_empty()
}

// --- bitrate: the low line ---

#[test]
fn a_lossy_file_below_190_kbps_is_low_bitrate_and_at_190_it_is_not() {
    let stored = row();
    let low = mp3(189, &stored);
    assert_eq!(low.verdict, Some(Verdict::LowBitrate));
    assert_eq!(low.findings, [Finding::LowBitrate { kbps: 189 }]);
    assert_eq!(low.reason(), Some(&Finding::LowBitrate { kbps: 189 }));

    let edge = mp3(190, &stored);
    assert_eq!(flagged(&edge), None);
    assert_eq!(edge.verdict, Some(Verdict::Ok));
}

#[test]
fn a_nominal_192_that_averages_191_is_not_flagged() {
    let a = mp3(191, &row());
    assert_eq!(flagged(&a), None);
    assert_eq!(
        a.note(),
        Some(&Finding::CouldBeBetter {
            kbps: 191,
            full_kbps: 318
        })
    );
}

#[test]
fn the_low_line_is_the_same_for_aac_and_the_other_lossy_codecs() {
    let stored = row();
    for codec in [
        Codec::Aac,
        Codec::Vorbis,
        Codec::Opus,
        Codec::Wma,
        Codec::Ac3,
        Codec::Eac3,
        Codec::Musepack,
    ] {
        assert_eq!(
            assess_row(codec, 189, &stored).verdict,
            Some(Verdict::LowBitrate),
            "{codec:?}"
        );
        assert_eq!(flagged(&assess_row(codec, 190, &stored)), None, "{codec:?}");
    }
}

// --- bitrate: full quality and the "could be better" note ---

#[test]
fn an_mp3_is_could_be_better_below_318_kbps_and_at_full_quality_from_318() {
    let stored = row();
    let below = mp3(317, &stored);
    assert_eq!(below.verdict, Some(Verdict::Ok));
    assert_eq!(
        below.findings,
        [Finding::CouldBeBetter {
            kbps: 317,
            full_kbps: 318
        }]
    );
    assert!(is_ok_without_notes(&mp3(318, &stored)));
    assert!(is_ok_without_notes(&mp3(320, &stored)));
}

#[test]
fn an_aac_is_could_be_better_below_240_kbps_and_at_full_quality_from_240() {
    let stored = row();
    let below = assess_row(Codec::Aac, 239, &stored);
    assert_eq!(below.verdict, Some(Verdict::Ok));
    assert_eq!(
        below.findings,
        [Finding::CouldBeBetter {
            kbps: 239,
            full_kbps: 240
        }]
    );
    assert!(is_ok_without_notes(&assess_row(Codec::Aac, 240, &stored)));
    // An MP3 at the same rate is still short of full quality.
    assert!(mp3(240, &stored).note().is_some());
}

#[test]
fn vorbis_opus_and_wma_reach_full_quality_at_the_mp3_line() {
    let stored = row();
    for codec in [Codec::Vorbis, Codec::Opus, Codec::Wma] {
        assert!(
            assess_row(codec, 317, &stored).note().is_some(),
            "{codec:?}"
        );
        assert!(
            is_ok_without_notes(&assess_row(codec, 318, &stored)),
            "{codec:?}"
        );
    }
}

#[test]
fn a_vbr_v0_average_of_245_is_could_be_better() {
    let a = mp3(245, &row());
    assert_eq!(a.verdict, Some(Verdict::Ok));
    assert!(matches!(
        a.note(),
        Some(Finding::CouldBeBetter { kbps: 245, .. })
    ));
}

#[test]
fn a_lossless_file_has_no_bitrate_level() {
    let stored = row();
    for codec in [
        Codec::Flac,
        Codec::Alac,
        Codec::Pcm,
        Codec::PcmFloat,
        Codec::WavPack,
        Codec::Ape,
    ] {
        for kbps in [1, 189, 900] {
            let a = assess_row(codec, kbps, &stored);
            assert!(is_ok_without_notes(&a), "{codec:?} {kbps}: {a:?}");
        }
    }
}

#[test]
fn codecs_with_nothing_to_judge_them_by_get_no_bitrate_finding() {
    let stored = row();
    for codec in [
        Codec::Speex,
        Codec::AdpcmMs,
        Codec::AdpcmIma,
        Codec::Alaw,
        Codec::Mulaw,
        Codec::Other,
    ] {
        let a = assess_row(codec, 8, &stored);
        assert!(is_ok_without_notes(&a), "{codec:?}: {a:?}");
    }
}

#[test]
fn a_file_with_no_known_bitrate_or_codec_gets_no_bitrate_finding() {
    let stored = row();
    let facts = |codec, bitrate_kbps| Facts {
        codec,
        bitrate_kbps,
        sample_rate_hz: Some(44_100),
        read: None,
        audio_hash: Some(&HASH[..]),
        stored: Some(&stored),
    };
    assert!(is_ok_without_notes(&assess(&facts(Some(Codec::Mp3), None))));
    assert!(is_ok_without_notes(&assess(&facts(None, Some(64)))));
    assert!(is_ok_without_notes(&assess(&facts(
        Some(Codec::Mp3),
        Some(0)
    ))));
}

// --- suspect transcode ---

#[test]
fn an_mp3_at_254_kbps_or_more_with_a_cutoff_below_17_khz_is_a_suspect_transcode() {
    let a = mp3(320, &with_cutoff(16_000));
    assert_eq!(a.verdict, Some(Verdict::SuspectTranscode));
    assert_eq!(
        a.findings,
        [Finding::SuspectTranscodeLossy {
            cutoff_hz: 16_000,
            kbps: 320
        }]
    );
}

#[test]
fn the_mp3_cutoff_line_is_17_khz_exactly() {
    assert_eq!(
        mp3(320, &with_cutoff(16_999)).verdict,
        Some(Verdict::SuspectTranscode)
    );
    // At the line it's a note, not a flag.
    let at = mp3(320, &with_cutoff(17_000));
    assert_eq!(at.verdict, Some(Verdict::Ok));
    assert!(matches!(
        at.note(),
        Some(Finding::PossiblyLowerQuality {
            cutoff_hz: 17_000,
            ..
        })
    ));
}

#[test]
fn the_mp3_bitrate_line_for_a_transcode_is_254_kbps_exactly() {
    let stored = with_cutoff(16_000);
    assert_eq!(mp3(256, &stored).verdict, Some(Verdict::SuspectTranscode));
    // A nominal 256 that averages 255 is still checked.
    assert_eq!(mp3(255, &stored).verdict, Some(Verdict::SuspectTranscode));
    assert_eq!(mp3(254, &stored).verdict, Some(Verdict::SuspectTranscode));
    // 253: a low cutoff is what the encoder does at that rate.
    let below = mp3(253, &stored);
    assert_eq!(below.verdict, Some(Verdict::Ok));
    assert!(matches!(below.note(), Some(Finding::CouldBeBetter { .. })));
    assert_eq!(below.findings.len(), 1);
    // And a 128 kbps file is low bitrate, not a transcode.
    assert_eq!(mp3(128, &stored).verdict, Some(Verdict::LowBitrate));
}

#[test]
fn only_mp3_is_judged_by_the_lossy_cutoff_line() {
    // AAC and Vorbis at a high rate with a low cutoff: no transcode finding.
    let stored = with_cutoff(16_000);
    for codec in [Codec::Aac, Codec::Vorbis, Codec::Opus, Codec::Wma] {
        let a = assess_row(codec, 320, &stored);
        assert_eq!(a.verdict, Some(Verdict::Ok), "{codec:?}");
    }
}

#[test]
fn a_lossless_file_with_a_cutoff_below_19_5_khz_is_a_suspect_transcode() {
    for codec in [Codec::Flac, Codec::Pcm, Codec::PcmFloat, Codec::Alac] {
        let a = lossless(codec, &with_cutoff(19_499));
        assert_eq!(a.verdict, Some(Verdict::SuspectTranscode), "{codec:?}");
        assert_eq!(
            a.findings,
            [Finding::SuspectTranscodeLossless { cutoff_hz: 19_499 }]
        );
        assert!(
            is_ok_without_notes(&lossless(codec, &with_cutoff(19_500))),
            "{codec:?}"
        );
    }
}

#[test]
fn a_lossless_cutoff_between_17_and_19_5_khz_is_flagged_not_noted() {
    let a = lossless(Codec::Flac, &with_cutoff(18_000));
    assert_eq!(a.verdict, Some(Verdict::SuspectTranscode));
    assert!(a.note().is_none());
}

#[test]
fn a_sample_rate_with_no_room_above_the_line_is_never_a_transcode() {
    let stored = with_cutoff(15_000);
    let at = |codec, rate| {
        assess(&Facts {
            codec: Some(codec),
            bitrate_kbps: Some(900),
            sample_rate_hz: rate,
            read: None,
            audio_hash: Some(&HASH[..]),
            stored: Some(&stored),
        })
    };
    // 39 000 Hz: Nyquist 19 500, the line itself.
    assert_eq!(at(Codec::Flac, Some(39_000)).verdict, Some(Verdict::Ok));
    assert_eq!(
        at(Codec::Flac, Some(39_002)).verdict,
        Some(Verdict::SuspectTranscode)
    );
    assert_eq!(at(Codec::Flac, Some(32_000)).verdict, Some(Verdict::Ok));
    assert_eq!(
        at(Codec::Flac, Some(44_100)).verdict,
        Some(Verdict::SuspectTranscode)
    );
    // An unknown rate can't be vouched for.
    assert_eq!(at(Codec::Flac, None).verdict, Some(Verdict::Ok));
    // An MP3 at 320 kbps: the line is 17 kHz, so 34 000 Hz.
    let mp3_at = |rate| {
        assess(&Facts {
            codec: Some(Codec::Mp3),
            bitrate_kbps: Some(320),
            sample_rate_hz: Some(rate),
            read: None,
            audio_hash: Some(&HASH[..]),
            stored: Some(&stored),
        })
    };
    assert_eq!(mp3_at(32_000).verdict, Some(Verdict::Ok));
    assert_eq!(mp3_at(34_000).verdict, Some(Verdict::Ok));
    assert_eq!(mp3_at(34_002).verdict, Some(Verdict::SuspectTranscode));
    assert_eq!(mp3_at(48_000).verdict, Some(Verdict::SuspectTranscode));
}

#[test]
fn a_cutoff_that_couldnt_be_measured_is_no_finding_and_not_a_flag() {
    for gap in [Gap::TooShort, Gap::Silent, Gap::LowRate] {
        let stored = Stored {
            cutoff_hz: None,
            cutoff_gap: Some(gap.as_str().to_owned()),
            ..row()
        };
        let a = mp3(320, &stored);
        assert!(is_ok_without_notes(&a), "{gap:?}: {a:?}");
        assert!(
            is_ok_without_notes(&lossless(Codec::Flac, &stored)),
            "{gap:?}"
        );
    }
}

// --- the "possibly lower quality" note ---

#[test]
fn a_cutoff_from_17_to_below_19_khz_is_possibly_lower_quality_at_254_kbps_or_more() {
    for hz in [17_000, 18_000, 18_999] {
        let a = mp3(320, &with_cutoff(hz));
        assert_eq!(a.verdict, Some(Verdict::Ok), "{hz}");
        assert_eq!(
            a.note(),
            Some(&Finding::PossiblyLowerQuality {
                cutoff_hz: hz as u32,
                kbps: 320
            }),
            "{hz}"
        );
    }
    assert!(is_ok_without_notes(&mp3(320, &with_cutoff(19_000))));
    assert!(is_ok_without_notes(&mp3(320, &with_cutoff(20_500))));
}

#[test]
fn the_possibly_lower_quality_note_needs_254_kbps_or_more() {
    let stored = with_cutoff(18_000);
    assert!(matches!(
        mp3(254, &stored).note(),
        Some(Finding::PossiblyLowerQuality { .. })
    ));
    // 192 kbps stops there by design: only the bitrate note shows.
    let a = mp3(253, &stored);
    assert!(a
        .findings
        .iter()
        .all(|f| !matches!(f, Finding::PossiblyLowerQuality { .. })));
}

#[test]
fn possibly_lower_quality_comes_before_could_be_better() {
    let a = mp3(256, &with_cutoff(18_000));
    assert_eq!(a.verdict, Some(Verdict::Ok));
    assert!(matches!(
        a.findings.as_slice(),
        [
            Finding::PossiblyLowerQuality { .. },
            Finding::CouldBeBetter { .. }
        ]
    ));
    assert!(matches!(
        a.note(),
        Some(Finding::PossiblyLowerQuality { .. })
    ));
}

// --- truncated ---

#[test]
fn audio_short_of_the_header_by_more_than_2_s_and_1_percent_is_truncated() {
    // 100 s header: 1% is 1 s, so the 2 s allowance is the one that binds.
    let at = assess_row(Codec::Flac, 900, &with_lengths(98_000, 100_000));
    assert_eq!(at.verdict, Some(Verdict::Ok));
    let over = assess_row(Codec::Flac, 900, &with_lengths(97_999, 100_000));
    assert_eq!(over.verdict, Some(Verdict::Truncated));
    assert_eq!(
        over.findings,
        [Finding::TruncatedShort {
            decoded_ms: 97_999,
            header_ms: 100_000
        }]
    );
}

#[test]
fn the_one_percent_allowance_binds_for_a_long_file() {
    // 600 s header: 1% is 6 s, and 2.5 s short passes the 2 s allowance.
    let under = assess_row(Codec::Flac, 900, &with_lengths(597_500, 600_000));
    assert_eq!(under.verdict, Some(Verdict::Ok));
    let at = assess_row(Codec::Flac, 900, &with_lengths(594_000, 600_000));
    assert_eq!(at.verdict, Some(Verdict::Ok));
    let over = assess_row(Codec::Flac, 900, &with_lengths(593_999, 600_000));
    assert_eq!(over.verdict, Some(Verdict::Truncated));
}

#[test]
fn audio_longer_than_the_header_or_with_no_header_length_is_not_truncated() {
    assert_eq!(
        assess_row(Codec::Flac, 900, &with_lengths(300_000, 240_000)).verdict,
        Some(Verdict::Ok)
    );
    let no_header = Stored {
        header_ms: None,
        decoded_ms: Some(10_000),
        ..row()
    };
    assert_eq!(
        assess_row(Codec::Flac, 900, &no_header).verdict,
        Some(Verdict::Ok)
    );
}

#[test]
fn a_stream_that_stopped_in_the_middle_is_truncated() {
    let a = mp3(320, &ended(Ended::CutShort));
    assert_eq!(a.verdict, Some(Verdict::Truncated));
    assert_eq!(a.findings, [Finding::TruncatedStream]);
}

#[test]
fn the_read_stage_saying_truncated_is_truncated() {
    let a = unmeasured(Some(Codec::Mp3), Some(320), Some(ReadVerdict::Truncated));
    assert_eq!(a.verdict, Some(Verdict::Truncated));
    assert_eq!(a.findings, [Finding::TruncatedFile]);
}

// --- broken ---

#[test]
fn the_read_stage_saying_broken_is_broken() {
    let a = unmeasured(Some(Codec::Mp3), Some(320), Some(ReadVerdict::Broken));
    assert_eq!(a.verdict, Some(Verdict::Broken));
    assert_eq!(a.findings, [Finding::BrokenUnplayable]);
    // With no codec or bitrate either.
    assert_eq!(
        unmeasured(None, None, Some(ReadVerdict::Broken)).verdict,
        Some(Verdict::Broken)
    );
}

#[test]
fn content_that_is_damaged_or_has_no_audio_is_broken() {
    let damaged = mp3(320, &failed(Failure::Damaged));
    assert_eq!(damaged.verdict, Some(Verdict::Broken));
    assert_eq!(damaged.findings, [Finding::BrokenDamaged]);
    let silent = mp3(320, &failed(Failure::NoAudio));
    assert_eq!(silent.verdict, Some(Verdict::Broken));
    assert_eq!(silent.findings, [Finding::BrokenNoAudio]);
}

#[test]
fn a_stream_abandoned_after_too_many_errors_is_broken() {
    let a = mp3(320, &ended(Ended::GaveUp));
    assert_eq!(a.verdict, Some(Verdict::Broken));
    assert_eq!(a.findings, [Finding::BrokenGaveUp]);
}

#[test]
fn a_file_the_decoder_couldnt_handle_is_neither_broken_nor_ok() {
    for why in [
        Failure::UnsupportedFormat,
        Failure::UnsupportedCodec,
        Failure::DecoderCrashed,
    ] {
        let a = mp3(320, &failed(why));
        assert_eq!(a.verdict, None, "{why:?}");
        assert!(a.findings.is_empty(), "{why:?}");
    }
    // Header-only rules still apply to it.
    assert_eq!(
        mp3(128, &failed(Failure::UnsupportedCodec)).verdict,
        Some(Verdict::LowBitrate)
    );
}

// --- precedence and the findings list ---

#[test]
fn broken_beats_truncated_beats_suspect_transcode_and_the_findings_keep_that_order() {
    let stored = Stored {
        cutoff_hz: Some(16_000),
        ended: Some(Ended::CutShort.as_str().to_owned()),
        ..row()
    };
    let a = assess(&Facts {
        codec: Some(Codec::Mp3),
        bitrate_kbps: Some(320),
        sample_rate_hz: Some(44_100),
        read: Some(ReadVerdict::Broken),
        audio_hash: Some(&HASH[..]),
        stored: Some(&stored),
    });
    assert_eq!(a.verdict, Some(Verdict::Broken));
    assert_eq!(
        a.findings,
        [
            Finding::BrokenUnplayable,
            Finding::TruncatedStream,
            Finding::SuspectTranscodeLossy {
                cutoff_hz: 16_000,
                kbps: 320
            },
        ]
    );
    assert_eq!(a.reason(), Some(&Finding::BrokenUnplayable));
}

#[test]
fn truncated_beats_low_bitrate_and_both_are_listed() {
    let a = mp3(128, &ended(Ended::CutShort));
    assert_eq!(a.verdict, Some(Verdict::Truncated));
    assert_eq!(
        a.findings,
        [Finding::TruncatedStream, Finding::LowBitrate { kbps: 128 }]
    );
}

#[test]
fn a_flagged_file_has_no_notes() {
    let stored = Stored {
        decode_errors: 4,
        ..with_cutoff(16_000)
    };
    // Suspect transcode at 256 kbps: could-be-better and decode errors would
    // both apply as notes; neither is listed.
    let a = mp3(256, &stored);
    assert_eq!(a.verdict, Some(Verdict::SuspectTranscode));
    assert!(a.findings.iter().all(|f| !f.is_note()), "{a:?}");
    assert!(a.note().is_none());
    // Low bitrate: no could-be-better note beside it.
    let low = mp3(128, &stored);
    assert_eq!(low.findings, [Finding::LowBitrate { kbps: 128 }]);
}

#[test]
fn a_clean_file_at_full_quality_is_ok_with_nothing_to_say() {
    let a = mp3(320, &row());
    assert!(is_ok_without_notes(&a));
    assert!(a.reason().is_none());
    assert!(a.note().is_none());
}

// --- decode errors ---

#[test]
fn decode_errors_in_a_complete_file_are_a_note_with_the_count_not_a_flag() {
    let stored = Stored {
        decode_errors: 3,
        ..row()
    };
    let a = mp3(320, &stored);
    assert_eq!(a.verdict, Some(Verdict::Ok));
    assert_eq!(a.findings, [Finding::DecodeErrors { count: 3 }]);
    assert!(is_ok_without_notes(&mp3(320, &row())));
}

#[test]
fn decode_errors_are_noted_before_the_quality_notes() {
    let stored = Stored {
        decode_errors: 1,
        ..with_cutoff(18_000)
    };
    let a = mp3(256, &stored);
    assert!(matches!(
        a.findings.as_slice(),
        [
            Finding::DecodeErrors { count: 1 },
            Finding::PossiblyLowerQuality { .. },
            Finding::CouldBeBetter { .. }
        ]
    ));
}

// --- no measurement ---

#[test]
fn a_file_that_wasnt_measured_has_no_verdict_and_is_never_ok() {
    for (codec, kbps) in [
        (Some(Codec::Mp3), Some(320)),
        (Some(Codec::Flac), Some(900)),
        (None, None),
    ] {
        let a = unmeasured(codec, kbps, None);
        assert_eq!(a.verdict, None);
        assert!(a.findings.is_empty());
        assert!(a.reason().is_none());
    }
}

#[test]
fn an_unmeasured_file_still_gets_low_bitrate_from_its_header() {
    let a = unmeasured(Some(Codec::Mp3), Some(128), None);
    assert_eq!(a.verdict, Some(Verdict::LowBitrate));
    assert_eq!(a.findings, [Finding::LowBitrate { kbps: 128 }]);
    // But not the could-be-better note: notes come with `ok`.
    let b = unmeasured(Some(Codec::Mp3), Some(256), None);
    assert_eq!(b.verdict, None);
    assert!(b.findings.is_empty());
}

#[test]
fn a_row_is_used_only_for_the_audio_and_the_method_it_was_made_by() {
    let transcode = with_cutoff(16_000);
    let facts = |hash: Option<&[u8]>, row: &Stored| {
        assess(&Facts {
            codec: Some(Codec::Mp3),
            bitrate_kbps: Some(320),
            sample_rate_hz: Some(44_100),
            read: None,
            audio_hash: hash,
            stored: Some(row),
        })
    };
    // The same hash, the current method: used.
    assert_eq!(
        facts(Some(&HASH[..]), &transcode).verdict,
        Some(Verdict::SuspectTranscode)
    );
    // The file has no audio hash: nothing vouches for the row.
    assert_eq!(facts(None, &transcode).verdict, None);
    // The audio changed since.
    assert_eq!(facts(Some(&[8u8; 34][..]), &transcode).verdict, None);
    // Another version of the method.
    let old = Stored {
        method_version: i64::from(VERSION) + 1,
        ..transcode
    };
    assert_eq!(facts(Some(&HASH[..]), &old).verdict, None);
}

#[test]
fn a_stale_row_does_not_hide_the_read_stages_verdict() {
    let stale = Stored {
        method_version: i64::from(VERSION) + 1,
        ..row()
    };
    let a = assess(&Facts {
        codec: Some(Codec::Mp3),
        bitrate_kbps: Some(320),
        sample_rate_hz: Some(44_100),
        read: Some(ReadVerdict::Truncated),
        audio_hash: Some(&HASH[..]),
        stored: Some(&stale),
    });
    assert_eq!(a.verdict, Some(Verdict::Truncated));
}

// --- names and serialization ---

#[test]
fn verdict_names_are_the_ones_the_file_table_accepts() {
    let names: Vec<_> = [
        Verdict::Ok,
        Verdict::LowBitrate,
        Verdict::SuspectTranscode,
        Verdict::Truncated,
        Verdict::Broken,
    ]
    .iter()
    .map(|v| v.as_str())
    .collect();
    assert_eq!(
        names,
        [
            "ok",
            "low_bitrate",
            "suspect_transcode",
            "truncated",
            "broken"
        ]
    );
}

#[test]
fn findings_serialize_as_a_camel_case_code_with_camel_case_numbers() {
    let json = |f: &Finding| serde_json::to_value(f).unwrap();
    assert_eq!(
        json(&Finding::TruncatedShort {
            decoded_ms: 90_000,
            header_ms: 100_000
        }),
        serde_json::json!({"code": "truncatedShort", "decodedMs": 90_000, "headerMs": 100_000})
    );
    assert_eq!(
        json(&Finding::SuspectTranscodeLossy {
            cutoff_hz: 16_000,
            kbps: 320
        }),
        serde_json::json!({"code": "suspectTranscodeLossy", "cutoffHz": 16_000, "kbps": 320})
    );
    assert_eq!(
        json(&Finding::CouldBeBetter {
            kbps: 245,
            full_kbps: 318
        }),
        serde_json::json!({"code": "couldBeBetter", "kbps": 245, "fullKbps": 318})
    );
    assert_eq!(
        json(&Finding::BrokenGaveUp),
        serde_json::json!({"code": "brokenGaveUp"})
    );
}

#[test]
fn an_assessment_serializes_its_verdict_and_findings() {
    let a = mp3(128, &row());
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::json!({
            "verdict": "lowBitrate",
            "findings": [{"code": "lowBitrate", "kbps": 128}]
        })
    );
}
