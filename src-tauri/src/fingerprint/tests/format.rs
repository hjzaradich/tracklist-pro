//! The stored form: it reads back exactly, carries its version, refuses to
//! compare across versions, and never panics on damaged bytes.

use proptest::prelude::*;

use super::audio;
use super::decode_bytes;
use crate::fingerprint::stored::CURRENT_PREFIX;
use crate::fingerprint::{compare, BlobError, CompareError, Comparison, Fingerprint, VERSION};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn any_fingerprint_reads_back_exactly_from_its_stored_bytes(
        items in proptest::collection::vec(any::<u32>(), 0..600),
    ) {
        let fp = Fingerprint::new(items.clone());
        let back = Fingerprint::from_blob(&fp.to_blob()).unwrap();
        prop_assert_eq!(back.items(), &items[..]);
        prop_assert_eq!(back.version(), VERSION);
    }

    #[test]
    fn slowly_changing_items_like_real_audio_also_read_back_exactly(
        start in any::<u32>(),
        flips in proptest::collection::vec(0u32..32, 0..400),
    ) {
        let mut item = start;
        let items: Vec<u32> = flips.iter().map(|&bit| { item ^= 1 << bit; item }).collect();
        let back = Fingerprint::from_blob(&Fingerprint::new(items.clone()).to_blob()).unwrap();
        prop_assert_eq!(back.items(), &items[..]);
    }

    #[test]
    fn random_bytes_are_refused_or_read_but_never_panic(
        bytes in proptest::collection::vec(any::<u8>(), 0..200),
        with_header in any::<bool>(),
    ) {
        let blob = if with_header {
            [CURRENT_PREFIX.as_slice(), &bytes].concat()
        } else {
            bytes
        };
        let _ = Fingerprint::from_blob(&blob);
    }

    #[test]
    fn a_stored_fingerprint_cut_short_or_with_extra_bytes_is_refused_as_damaged(
        items in proptest::collection::vec(any::<u32>(), 1..200),
        cut in 1usize..40,
    ) {
        let blob = Fingerprint::new(items).to_blob();
        let cut = cut.min(blob.len() - CURRENT_PREFIX.len());
        let short = &blob[..blob.len() - cut];
        prop_assert!(Fingerprint::from_blob(short).is_err());
        let long = [&blob[..], &[0xFF; 3]].concat();
        prop_assert_eq!(Fingerprint::from_blob(&long), Err(BlobError::Corrupt));
    }
}

#[test]
fn every_stored_fingerprint_starts_with_tlfp_and_its_version() {
    let blob = Fingerprint::new(vec![1, 2, 3]).to_blob();
    assert_eq!(&blob[..4], b"TLFP");
    assert_eq!(blob[4], VERSION);
    // Then chromaprint's own compressed form: algorithm 1 (preset_test2,
    // the one E3 graded) and the item count.
    assert_eq!(&blob[5..9], &[1, 0, 0, 3]);
}

#[test]
fn a_blob_from_another_version_is_refused_not_misread() {
    let mut blob = Fingerprint::new(vec![7; 50]).to_blob();
    blob[4] = VERSION + 1;
    assert_eq!(
        Fingerprint::from_blob(&blob),
        Err(BlobError::UnknownVersion(VERSION + 1))
    );
    assert_eq!(
        Fingerprint::from_blob(b"RIFF\x01\x02"),
        Err(BlobError::NotAFingerprint)
    );
    assert_eq!(Fingerprint::from_blob(b""), Err(BlobError::NotAFingerprint));
}

#[test]
fn fingerprints_made_by_different_versions_refuse_to_compare() {
    let song = decode_bytes(&audio::wav(&audio::pcm(1, 20.0, 22_050, 1))).unwrap();
    let mut older = song.clone();
    older.set_version_for_tests(VERSION - 1);
    assert_eq!(compare(&song, &older), Err(CompareError::Incomparable));
    assert!(compare(&song, &song).unwrap().is_duplicate());
}

#[test]
fn fingerprints_made_by_different_chromaprint_algorithms_refuse_to_compare() {
    let song = decode_bytes(&audio::wav(&audio::pcm(1, 20.0, 22_050, 1))).unwrap();
    let mut other = song.clone();
    other.set_algorithm_for_tests(2);
    assert_eq!(compare(&song, &other), Err(CompareError::Incomparable));
    assert_eq!(compare(&other, &song), Err(CompareError::Incomparable));
}

#[test]
fn a_duplicate_needs_90_percent_coverage_both_ways_and_a_score_of_4_or_less() {
    // ROADMAP 1.4, at its edges.
    for (coverage_a, coverage_b, score, duplicate) in [
        (0.9, 0.9, 4.0, true),
        (1.0, 1.0, 0.0, true),
        (0.89, 1.0, 0.0, false),
        (1.0, 0.89, 0.0, false),
        (1.0, 1.0, 4.01, false),
        (0.0, 0.0, 32.0, false),
    ] {
        let c = Comparison {
            coverage_a,
            coverage_b,
            score,
        };
        assert_eq!(c.is_duplicate(), duplicate, "{c:?}");
    }
}

#[test]
fn a_header_claiming_millions_of_items_is_refused_without_allocating_them() {
    let mut blob = CURRENT_PREFIX.to_vec();
    blob.extend_from_slice(&[1, 0xFF, 0xFF, 0xFF, 0, 0]);
    assert_eq!(Fingerprint::from_blob(&blob), Err(BlobError::Corrupt));
}

#[test]
fn a_songs_stored_fingerprint_is_smaller_than_its_raw_items() {
    let fp = decode_bytes(&audio::wav(&audio::pcm(3, 60.0, 22_050, 1))).unwrap();
    let raw = fp.items().len() * 4;
    let stored = fp.to_blob().len();
    println!(
        "60 s of audio: {} items, {raw} bytes raw, {stored} bytes stored ({:.1}x smaller)",
        fp.items().len(),
        raw as f64 / stored as f64
    );
    // These songs change chord every beat, so their items change a lot;
    // steadier music packs tighter.
    assert!(stored < raw, "{stored} bytes stored vs {raw} raw");
    // About 8 items a second of audio.
    assert!(
        (450..=500).contains(&fp.items().len()),
        "{}",
        fp.items().len()
    );
}
