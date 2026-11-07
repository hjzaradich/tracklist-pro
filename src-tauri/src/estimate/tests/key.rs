//! The key estimate (libKeyFinder's method), on audio made here: triads,
//! chord progressions, silence and awkward signals.

use super::{audio, source};
use crate::estimate::key::chroma::{pitch_classes, BANDS};
use crate::estimate::key::{self, Choice};
use crate::estimate::window::{measure, Measured};
use crate::tags::key::{Mode, MusicalKey};

pub(super) const RATE: u32 = 22_050;

/// The frequency of MIDI note `note` (A4 = 69 = 440 Hz).
fn hz(note: i32) -> f64 {
    440.0 * 2f64.powf(f64::from(note - 69) / 12.0)
}

/// The notes sounding together for `seconds`, each a sine with two softer
/// harmonics, mono at `rate`.
fn chord(notes: &[i32], seconds: f64, rate: u32) -> Vec<f64> {
    let frames = (seconds * f64::from(rate)) as usize;
    let gain = 0.6 / notes.len() as f64;
    (0..frames)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            notes
                .iter()
                .map(|&n| {
                    let w = std::f64::consts::TAU * hz(n) * t;
                    gain * (w.sin() + 0.4 * (2.0 * w).sin() + 0.2 * (3.0 * w).sin()) / 1.6
                })
                .sum()
        })
        .collect()
}

/// A triad on `root` (MIDI) in `mode`, voiced over three octaves.
fn triad(root: i32, mode: Mode) -> Vec<i32> {
    let third = if mode == Mode::Major { 4 } else { 3 };
    let mut notes = vec![root - 12];
    for octave in [0, 12] {
        notes.extend([root + octave, root + third + octave, root + 7 + octave]);
    }
    notes
}

/// I-IV-V-I in a major key, i-iv-V-i in a minor one (the V's leading
/// tone raised), on `tonic` (MIDI), each chord `seconds` long.
pub(super) fn progression(tonic: i32, mode: Mode, seconds: f64) -> Vec<f64> {
    let one = triad(tonic, mode);
    let four = triad(tonic + 5, mode);
    let five = triad(tonic + 7, Mode::Major);
    let mut audio = Vec::new();
    for notes in [&one, &four, &five, &one] {
        audio.extend(chord(notes, seconds, RATE));
    }
    audio
}

fn key_of(audio: &[f64]) -> Choice {
    key::of_audio(audio, RATE).expect("a rate the bands fit")
}

fn named(pitch_class: u8, mode: Mode) -> MusicalKey {
    MusicalKey { pitch_class, mode }
}

/// MIDI note in the third octave (C3 = 48) of pitch class `pc`.
pub(super) fn third_octave(pc: u8) -> i32 {
    48 + i32::from(pc)
}

#[test]
fn a_pure_major_or_minor_triad_is_its_own_key_in_several_keys() {
    for (pc, mode) in [
        (0, Mode::Major),
        (7, Mode::Major),
        (6, Mode::Major),
        (10, Mode::Major),
        (9, Mode::Minor),
        (4, Mode::Minor),
        (1, Mode::Minor),
        (5, Mode::Minor),
    ] {
        let audio = chord(&triad(third_octave(pc), mode), 12.0, RATE);
        let Choice::Key { key, confidence } = key_of(&audio) else {
            panic!("silence for {pc} {mode:?}");
        };
        assert_eq!(key, named(pc, mode), "{}", named(pc, mode).to_camelot());
        assert!(confidence > 0.0 && confidence <= 1.0, "{confidence}");
    }
}

#[test]
fn a_short_chord_progression_is_found_in_its_key_major_and_minor() {
    for (pc, mode) in [
        (2, Mode::Major),
        (8, Mode::Major),
        (11, Mode::Minor),
        (7, Mode::Minor),
    ] {
        let audio = progression(third_octave(pc), mode, 4.0);
        let Choice::Key { key, .. } = key_of(&audio) else {
            panic!("silence for {pc} {mode:?}");
        };
        assert_eq!(key, named(pc, mode), "{}", named(pc, mode).to_camelot());
    }
}

#[test]
fn a_progression_decoded_from_a_stereo_wav_through_the_window_gives_its_key() {
    let mono = progression(third_octave(2), Mode::Minor, 4.0);
    // Stereo, the right channel quieter: mixed to mono before the key.
    let mut samples = Vec::with_capacity(mono.len() * 2);
    for x in &mono {
        samples.push((x * 32_767.0).round() as i16);
        samples.push((x * 0.5 * 32_767.0).round() as i16);
    }
    let wav = audio::wav(&audio::Pcm {
        rate: RATE,
        channels: 2,
        samples,
    });
    let Measured::Window { key, .. } = measure(source(&wav), &mut |_| true).unwrap() else {
        panic!("not measured");
    };
    let Some(Choice::Key { key, .. }) = key else {
        panic!("no key: {key:?}");
    };
    assert_eq!(key, named(2, Mode::Minor));
}

#[test]
fn digital_silence_and_no_audio_at_all_are_silence() {
    assert_eq!(key_of(&vec![0.0; 10 * RATE as usize]), Choice::Silence);
    assert_eq!(key_of(&[]), Choice::Silence);
}

#[test]
fn awkward_signals_give_a_finite_answer_and_never_panic() {
    let finite = |choice: Choice| match choice {
        Choice::Key { confidence, .. } => {
            confidence.is_finite() && (0.0..=1.0).contains(&confidence)
        }
        Choice::Silence => true,
    };
    // Very short: less than one frame, then one sample.
    assert!(finite(key_of(&chord(&[57, 61, 64], 0.5, RATE))));
    assert!(finite(key_of(&[0.3])));
    // A constant offset with no tone.
    assert!(finite(key_of(&vec![0.5; 10 * RATE as usize])));
    // Hard clipping: a loud triad squared off at full scale keeps its key.
    let clipped: Vec<f64> = chord(&triad(third_octave(9), Mode::Minor), 12.0, RATE)
        .into_iter()
        .map(|x| (x * 20.0).clamp(-1.0, 1.0))
        .collect();
    let choice = key_of(&clipped);
    assert!(finite(choice));
    assert!(
        matches!(choice, Choice::Key { key, .. } if key == named(9, Mode::Minor)),
        "{choice:?}"
    );
    // Not-a-number and infinity count as silence rather than poisoning it.
    let mut broken = chord(&triad(third_octave(0), Mode::Major), 12.0, RATE);
    broken[1000] = f64::NAN;
    broken[2000] = f64::INFINITY;
    let choice = key_of(&broken);
    assert!(
        matches!(choice, Choice::Key { key, .. } if key == named(0, Mode::Major)),
        "{choice:?}"
    );
    // A rate too low for the bands has no key at all.
    assert_eq!(key::of_audio(&[0.1; 1000], 4_000), None);
}

/// The chroma of a fixed signal: an A minor triad with harmonics, 12 s at
/// 22,050 Hz. If an edit to the method moves any band, this fails.
#[allow(clippy::excessive_precision)]
const GOLDEN: [f64; BANDS] = [
    3.014310121698054,
    3.2157188618269266,
    3.43388098335965,
    3.6710297728597334,
    3.9297521912721427,
    4.213211537564379,
    4.525488743407548,
    4.871456733252648,
    5.257622629359832,
    5.692482025342585,
    6.187425767608106,
    6.758140070432238,
    7.426925564156505,
    8.227186696607552,
    9.21066770539011,
    10.463086727487502,
    12.137581200052786,
    14.539991592923082,
    18.383376897332123,
    25.829129571125655,
    48.214059273440455,
    4553.852919874705,
    37.305943743150166,
    14.613712147868362,
    6.839748178059484,
    2.524578900447909,
    0.62982771456785,
    3.231652712649538,
    5.94830122056446,
    9.247795219058078,
    14.119254263824375,
    23.466018729570756,
    52.71903832247493,
    6676.628592274885,
    77.36687152497807,
    68.07029235572108,
    4805.354431984979,
    31.950624285436238,
    9.759010589205584,
    11.846992235476282,
    5046.552400470872,
    63.21238020232208,
    52.29247866742237,
    59.115500825916996,
    90.71569762140331,
    6899.398792635448,
    17.855039778068882,
    48.94660783250542,
    6942.294154063617,
    49.49660960951116,
    11.209812904769425,
    44.476871081751575,
    7673.450154004503,
    83.22514516010398,
    48.13608011245291,
    1008.1089259107398,
    6.180279404528803,
    2029.280079002323,
    30.118011436602497,
    1016.2893928892267,
    2043.3568647254774,
    23.127475982104766,
    10.500323271191057,
    10.482104219400824,
    2989.072906740947,
    34.66617693570869,
    16.892113921126644,
    1029.9442061488069,
    26.03615391701616,
    19.009437837426063,
    13.99404032071461,
    600.8079077759265,
];

#[test]
fn the_chroma_of_a_fixed_signal_matches_its_golden_values() {
    let audio = chord(&triad(57, Mode::Minor), 12.0, RATE);
    let chroma = key::chroma_of_audio(&audio, RATE).unwrap();
    let scale = chroma.iter().cloned().fold(0.0, f64::max);
    let drifted: Vec<String> = chroma
        .iter()
        .zip(&GOLDEN)
        .enumerate()
        .filter(|(_, (a, b))| (*a - *b).abs() > 1e-9 * scale)
        .map(|(i, (a, b))| format!("band {i}: {a:e}, golden {b:e}"))
        .collect();
    assert!(
        drifted.is_empty(),
        "{}\nall: {:?}",
        drifted.join("\n"),
        chroma
    );
    // The folded classes: A, C and E stand out.
    let classes = pitch_classes(&chroma);
    let mut order: Vec<usize> = (0..12).collect();
    order.sort_by(|a, b| classes[*b].total_cmp(&classes[*a]));
    let mut top: Vec<usize> = order[..3].to_vec();
    top.sort();
    assert_eq!(top, [0, 4, 9]);
}
