//! Matching (ROADMAP 1.4, E3): the same audio in any format, rate or
//! bitrate compares as the same; different audio doesn't; and because the
//! fingerprint covers the whole track, a cut or a late difference shows.

use super::audio::{self, Pcm};
use super::decode_bytes;
use crate::fingerprint::{compare, Comparison, Fingerprint};

const SECONDS: f64 = 40.0;

fn print(bytes: &[u8]) -> Fingerprint {
    decode_bytes(bytes).expect("the test audio decodes")
}

fn compared(a: &Fingerprint, b: &Fingerprint) -> Comparison {
    compare(a, b).expect("same version")
}

/// Song `seed` in every format and several rates, channel counts and
/// bitrates, named for messages.
fn renditions(seed: u64) -> Vec<(&'static str, Vec<u8>)> {
    let cd = audio::pcm(seed, SECONDS, 44_100, 2);
    let mono = |rate| audio::pcm(seed, SECONDS, rate, 1);
    #[allow(unused_mut)]
    let mut all = vec![
        ("WAV 44.1 kHz stereo", audio::wav(&cd)),
        ("FLAC 44.1 kHz mono", audio::flac(&mono(44_100))),
        (
            "FLAC 48 kHz stereo",
            audio::flac(&audio::pcm(seed, SECONDS, 48_000, 2)),
        ),
        ("AIFF 22.05 kHz mono", audio::aiff(&mono(22_050))),
        ("M4A (ALAC) 32 kHz mono", audio::m4a_alac(&mono(32_000))),
    ];
    #[cfg(windows)]
    all.extend([
        ("MP3 320 kbps stereo", audio::mp3(&cd, 320)),
        ("MP3 128 kbps stereo", audio::mp3(&cd, 128)),
        ("MP3 96 kbps 22.05 kHz mono", audio::mp3(&mono(22_050), 96)),
    ]);
    all
}

#[test]
fn the_same_song_as_wav_flac_aiff_alac_and_mp3_at_any_rate_or_bitrate_compares_as_the_same() {
    let all = renditions(11);
    let (reference_name, reference) = (&all[0].0, print(&all[0].1));
    for (name, bytes) in &all[1..] {
        let other = print(bytes);
        let c = compared(&reference, &other);
        println!("{reference_name} vs {name}: {c:?}");
        assert!(
            c.is_duplicate(),
            "{name} should match {reference_name}: {c:?}"
        );
    }
}

#[test]
fn different_songs_never_compare_as_the_same_whatever_the_format() {
    let songs: Vec<Fingerprint> = (21..26)
        .map(|seed| print(&audio::wav(&audio::pcm(seed, SECONDS, 44_100, 2))))
        .collect();
    for (i, a) in songs.iter().enumerate() {
        for b in &songs[i + 1..] {
            let c = compared(a, b);
            assert!(!c.is_duplicate(), "{c:?}");
            assert!(c.coverage_a < 0.5 && c.coverage_b < 0.5, "{c:?}");
        }
    }
    // Across formats too: one song's MP3 (or AIFF) against another's FLAC.
    #[cfg(windows)]
    let lossy = print(&audio::mp3(&audio::pcm(21, SECONDS, 44_100, 2), 128));
    #[cfg(not(windows))]
    let lossy = print(&audio::aiff(&audio::pcm(21, SECONDS, 44_100, 2)));
    let flac = print(&audio::flac(&audio::pcm(22, SECONDS, 44_100, 1)));
    let c = compared(&lossy, &flac);
    assert!(!c.is_duplicate(), "{c:?}");
}

#[test]
fn a_radio_edit_cut_from_an_extended_mix_is_covered_one_way_only() {
    let extended: Pcm = audio::pcm(31, 90.0, 22_050, 1);
    let edit = extended.slice(20.0, 60.0);
    let (extended, edit) = (print(&audio::wav(&extended)), print(&audio::flac(&edit)));
    let c = compared(&edit, &extended);
    println!("edit vs extended: {c:?}");
    assert!(c.coverage_a >= 0.9, "the edit is all inside the mix: {c:?}");
    assert!(
        c.coverage_b < 0.6,
        "most of the mix isn't in the edit: {c:?}"
    );
    assert!(
        !c.is_duplicate(),
        "a cut links, it doesn't merge (1.4): {c:?}"
    );
    // The other way round: the same answer, sides swapped.
    let back = compared(&extended, &edit);
    println!("extended vs edit: {back:?}");
    assert!(back.coverage_b >= 0.9, "{back:?}");
    assert!(back.coverage_a < 0.6, "{back:?}");
    assert!(!back.is_duplicate(), "{back:?}");
}

#[test]
fn two_files_alike_for_the_first_minute_but_not_after_are_not_the_same() {
    // E3 pair #42: aligned for about two minutes, then nothing. Only a
    // full-track fingerprint sees it (ROADMAP 1.4).
    let opening = audio::song(41, 60.0, 22_050);
    let a = [opening.clone(), audio::song(42, 60.0, 22_050)].concat();
    let b = [opening, audio::song(43, 60.0, 22_050)].concat();
    let to_pcm = |x: &[f64]| Pcm {
        rate: 22_050,
        channels: 1,
        samples: x.iter().map(|s| (s * 30_000.0) as i16).collect(),
    };
    let (a, b) = (
        print(&audio::wav(&to_pcm(&a))),
        print(&audio::wav(&to_pcm(&b))),
    );
    let c = compared(&a, &b);
    println!("same opening, different ending: {c:?}");
    assert!(c.coverage_a > 0.3, "the opening matches: {c:?}");
    assert!(!c.is_duplicate(), "{c:?}");
}
