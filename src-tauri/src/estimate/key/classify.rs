// Translated from libKeyFinder keyclassifier.cpp, toneprofiles.cpp and the
// tone profiles of constants.cpp (https://github.com/mixxxdj/libkeyfinder).
// Copyright 2011-2015 Ibrahim Sha'ath, and the Mixxx contributors.
// The profiles are from Sha'ath's 2011 thesis.
// libKeyFinder is free software under the GNU General Public License,
// version 3 or (at your option) any later version; so is this translation.
// Translated to Rust and modified by the tracklist-pro contributors, 2026-11.

//! Choosing the key: the chroma's cosine similarity with KeyFinder's major
//! and minor profiles, turned to each of the 12 tonics; the best of the 24
//! wins, and a chroma that matches none (all zeros) is silence.
//!
//! As libKeyFinder does it, the comparison is over all 72 bands: each
//! profile is the 12-semitone shape repeated over the six octaves, each
//! octave weighted by its own factor, so a key is matched octave by octave,
//! not on the chroma folded to 12 pitch classes.
//!
//! What libKeyFinder doesn't give: a **confidence**, here how far the best
//! score stands above the runner-up's (0 to 1).

use super::chroma::{BANDS, OCTAVES, SEMITONES};
use crate::tags::key::{Mode, MusicalKey};

/// KeyFinder's major profile, tonic first.
#[allow(clippy::excessive_precision)]
const MAJOR: [f64; SEMITONES] = [
    7.23900502618145225142,
    3.50351166725158691406,
    3.58445177536649417505,
    2.84511816478676315967,
    5.81898892118549859731,
    4.55865057415321039969,
    2.44778850545506543313,
    6.99473192146829525484,
    3.39106613673504853068,
    4.55614256655143456953,
    4.07392666663523606019,
    4.45932757378886890365,
];

/// KeyFinder's minor profile, tonic first.
#[allow(clippy::excessive_precision)]
const MINOR: [f64; SEMITONES] = [
    7.00255045060284420089,
    3.14360279015996679775,
    4.35904319714962529275,
    5.40418120718934069657,
    3.67234420879306133756,
    4.08971184917797891956,
    3.90791435991553992579,
    6.19960288562316463867,
    3.63424625625277419871,
    2.87241191079875557435,
    5.35467999794542670600,
    3.83242038595048351013,
];

/// How much each octave counts, lowest first.
#[allow(clippy::excessive_precision)]
const OCTAVE_WEIGHTS: [f64; OCTAVES] = [
    0.39997267549999998559,
    0.55634425248300645173,
    0.52496636345143543600,
    0.60847548384277727607,
    0.59898115679999996974,
    0.49072435317960994006,
];

/// The pitch class (C = 0) of the tonic the profiles start from: A.
const FIRST_TONIC: usize = 9;

/// What the classifier chose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Choice {
    Key { key: MusicalKey, confidence: f64 },
    Silence,
}

/// `profile` over the six octaves, each weighted (`toneProfileMajor`).
fn weighted(profile: &[f64; SEMITONES]) -> [f64; BANDS] {
    let mut out = [0.0; BANDS];
    for (o, weight) in OCTAVE_WEIGHTS.iter().enumerate() {
        for (s, value) in profile.iter().enumerate() {
            out[o * SEMITONES + s] = weight * value;
        }
    }
    out
}

/// The cosine similarity of `chroma` (bands from C) with `profile` turned
/// so its tonic is `offset` semitones above A (`ToneProfile::cosineSimilarity`).
fn cosine(chroma: &[f64; BANDS], profile: &[f64; BANDS], offset: usize) -> f64 {
    let mut intersection = 0.0;
    let mut profile_norm = 0.0;
    let mut input_norm = 0.0;
    for o in 0..OCTAVES {
        for k in 0..SEMITONES {
            // Band k of the octave (C = 0) reads the profile at its
            // distance above the tonic.
            let p = profile[o * SEMITONES + (k + SEMITONES + 3 - offset) % SEMITONES];
            let x = chroma[o * SEMITONES + k];
            intersection += x * p;
            profile_norm += p * p;
            input_norm += x * x;
        }
    }
    if profile_norm > 0.0 && input_norm > 0.0 {
        intersection / (profile_norm.sqrt() * input_norm.sqrt())
    } else {
        0.0
    }
}

/// The 24 scores, in libKeyFinder's order: A major, A minor, B♭ major, …
pub fn scores(chroma: &[f64; BANDS]) -> [f64; 24] {
    let major = weighted(&MAJOR);
    let minor = weighted(&MINOR);
    let mut scores = [0.0; 24];
    for i in 0..SEMITONES {
        scores[i * 2] = cosine(chroma, &major, i);
        scores[i * 2 + 1] = cosine(chroma, &minor, i);
    }
    scores
}

/// The key of `chroma`: the best score above silence's (which is zero), the
/// first of equals winning (`KeyClassifier::classify`).
pub fn classify(chroma: &[f64; BANDS]) -> Choice {
    let scores = scores(chroma);
    let mut best = 0.0;
    let mut choice = None;
    for (i, &score) in scores.iter().enumerate() {
        if score > best {
            best = score;
            choice = Some(i);
        }
    }
    let Some(i) = choice else {
        return Choice::Silence;
    };
    let runner_up = scores
        .iter()
        .enumerate()
        .filter(|&(j, _)| j != i)
        .map(|(_, &s)| s)
        .fold(0.0, f64::max);
    Choice::Key {
        key: MusicalKey {
            pitch_class: ((FIRST_TONIC + i / 2) % SEMITONES) as u8,
            mode: if i % 2 == 0 { Mode::Major } else { Mode::Minor },
        },
        confidence: (best - runner_up).clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chroma with energy only in the given pitch classes, every octave.
    fn tones(classes: &[usize]) -> [f64; BANDS] {
        let mut c = [0.0; BANDS];
        for o in 0..OCTAVES {
            for &k in classes {
                c[o * SEMITONES + k] = 1.0;
            }
        }
        c
    }

    #[test]
    fn a_profile_shaped_chroma_is_its_own_key_for_every_tonic() {
        for tonic in 0..SEMITONES {
            for (mode, profile) in [(Mode::Major, &MAJOR), (Mode::Minor, &MINOR)] {
                let mut c = [0.0; BANDS];
                for (band, slot) in c.iter_mut().enumerate() {
                    let k = band % SEMITONES;
                    *slot = profile[(k + SEMITONES - tonic) % SEMITONES];
                }
                let Choice::Key { key, confidence } = classify(&c) else {
                    panic!("silence");
                };
                assert_eq!(
                    key,
                    MusicalKey {
                        pitch_class: tonic as u8,
                        mode
                    }
                );
                assert!(confidence > 0.0);
            }
        }
    }

    #[test]
    fn a_c_major_triad_is_c_major_and_nothing_is_silence() {
        let Choice::Key { key, .. } = classify(&tones(&[0, 4, 7])) else {
            panic!("silence");
        };
        assert_eq!(
            key,
            MusicalKey {
                pitch_class: 0,
                mode: Mode::Major
            }
        );
        assert_eq!(classify(&[0.0; BANDS]), Choice::Silence);
    }
}
