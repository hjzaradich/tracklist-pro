//! A small made-up library with known answers, mirroring the duplicate and
//! version cases of `tools/fixture-gen`'s ground truth: one recording in
//! many formats, cuts that are exact slices of a longer mix, Clean and
//! Dirty, reworks that share a little or nothing, and unrelated songs.
//!
//! On top of those, the shapes E3 measured on real files and the generator
//! doesn't make (1bA-4; `spikes/results/e3-summary.md`): cuts that aren't
//! clean slices, a rework that keeps scattered pieces, and another
//! recording of a song that lines up only weakly. [`super::e3_patterns`]
//! checks where each lands, and says what can't be made up at all.
//!
//! It's fingerprinted once and shared by the tests that use it.

use std::sync::OnceLock;

use crate::fingerprint::Fingerprint;

use super::audio;
use super::{cut, pcm, print, RATE};

pub struct File {
    pub name: &'static str,
    pub fingerprint: Fingerprint,
}

/// `inner` holds the audio of `outer`, starting `at_seconds` in.
pub struct Contains {
    pub outer: &'static str,
    pub inner: &'static str,
    pub at_seconds: f64,
}

pub struct Corpus {
    pub files: Vec<File>,
    /// Groups of files holding the same recording: every pair in a group
    /// is a duplicate.
    pub duplicate_groups: Vec<Vec<&'static str>>,
    /// Cuts that are exact slices.
    pub contains: Vec<Contains>,
    /// Cuts as E3 measured real ones, `(shorter, longer)`: most of the
    /// shorter file is in the longer, but in more than one piece.
    pub rough_cuts: Vec<(&'static str, &'static str)>,
    /// Cuts of the same length: the same production with a few words
    /// muted.
    pub clean_dirty: Vec<(&'static str, &'static str)>,
    /// Reworks sharing a good part of the audio (a VIP keeping the first
    /// half of the original, a rework keeping scattered pieces).
    pub reworks_sharing_audio: Vec<(&'static str, &'static str)>,
    /// Another recording of the same song: it lines up with the first one
    /// for a good part, but so weakly that hardly an item is the same bit
    /// for bit. Comparing the two sees it; the block index has nothing to
    /// propose them by.
    pub weak_matches: Vec<(&'static str, &'static str)>,
    /// Related pairs sharing little or nothing a fingerprint can see.
    pub other_reworks: Vec<(&'static str, &'static str)>,
}

impl Corpus {
    pub fn fingerprint(&self, name: &str) -> &Fingerprint {
        &self.files[self.index_of(name)].fingerprint
    }

    pub fn index_of(&self, name: &str) -> usize {
        self.files
            .iter()
            .position(|f| f.name == name)
            .unwrap_or_else(|| panic!("no corpus file {name}"))
    }

    /// Every pair of files holding the same recording.
    pub fn duplicate_pairs(&self) -> Vec<(&'static str, &'static str)> {
        let mut pairs = Vec::new();
        for group in &self.duplicate_groups {
            for (i, a) in group.iter().enumerate() {
                for b in &group[i + 1..] {
                    pairs.push((*a, *b));
                }
            }
        }
        pairs
    }

    /// Every pair the matching pass must find: duplicates, cuts,
    /// Clean/Dirty and the reworks sharing audio.
    pub fn true_pairs(&self) -> Vec<(&'static str, &'static str)> {
        let mut pairs = self.duplicate_pairs();
        pairs.extend(self.contains.iter().map(|c| (c.outer, c.inner)));
        pairs.extend(self.rough_cuts.iter().copied());
        pairs.extend(self.clean_dirty.iter().copied());
        pairs.extend(self.reworks_sharing_audio.iter().copied());
        pairs
    }

    /// Whether `a` and `b` are related in any way the corpus records.
    pub fn related(&self, a: &str, b: &str) -> bool {
        self.true_pairs()
            .into_iter()
            .chain(self.other_reworks.iter().copied())
            .chain(self.weak_matches.iter().copied())
            .any(|(x, y)| (x == a && y == b) || (x == b && y == a))
    }
}

pub fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(build)
}

const GLASSWING: u64 = 101;
const HARBOR: u64 = 202;
const MOTH: u64 = 305;

pub const EXTENDED: &str = "harbor (extended).flac";
pub const ORIGINAL: &str = "harbor (original).flac";
pub const RADIO: &str = "harbor (radio edit)";
pub const VIP: &str = "harbor (vip).wav";
pub const REMIX: &str = "harbor (remix).wav";
pub const LIVE: &str = "harbor (live).wav";
pub const DIRTY: &str = "moth (dirty).flac";
pub const CLEAN: &str = "moth (clean).flac";
pub const MASHUP: &str = "harbor x moth (mashup).wav";

pub const RIDGE_ORIGINAL: &str = "ridge (original).flac";
pub const RIDGE_EXTENDED: &str = "ridge (extended).wav";
pub const QUAY_ORIGINAL: &str = "quay (original).flac";
pub const QUAY_RADIO: &str = "quay (radio edit)";
pub const FEN_ORIGINAL: &str = "fen (original).wav";
pub const FEN_REWORK: &str = "fen (rework).wav";
/// The studio take and a demo of it, named alike, as E3's pair was.
pub const EMBER_STUDIO: &str = "ember.flac";
pub const EMBER_DEMO: &str = "ember.wav";

/// `samples` with song `seed` laid over them at `level` times their own
/// loudness: the same audio in another mix.
fn under(samples: &[f64], seed: u64, level: f64) -> Vec<f64> {
    let seconds = samples.len() as f64 / f64::from(RATE) + 1.0;
    samples
        .iter()
        .zip(audio::song(seed, seconds, RATE))
        .map(|(a, b)| (a + level * b) / (1.0 + level))
        .collect()
}

fn build() -> Corpus {
    let mut files: Vec<File> = Vec::new();
    let mut add = |name: &'static str, bytes: Vec<u8>| {
        files.push(File {
            name,
            fingerprint: print(&bytes),
        })
    };

    // ---- Duplicates: one recording, many files ---------------------------
    let cd = audio::pcm(GLASSWING, 60.0, 44_100, 2);
    let mono = audio::pcm(GLASSWING, 60.0, RATE, 1);
    add("glasswing.flac", audio::flac(&cd));
    add("glasswing.aiff", audio::aiff(&mono));
    add("glasswing.wav", audio::wav(&mono));
    add(
        "glasswing.m4a",
        audio::m4a_alac(&audio::pcm(GLASSWING, 60.0, 32_000, 1)),
    );
    // A rip with extra lead-in silence.
    let silence = vec![0.0; (1.5 * f64::from(RATE)) as usize];
    let lead_in = [silence, audio::song(GLASSWING, 60.0, RATE)].concat();
    add("glasswing (lead-in).flac", audio::flac(&pcm(&lead_in)));
    #[allow(unused_mut)]
    let mut glasswing = vec![
        "glasswing.flac",
        "glasswing.aiff",
        "glasswing.wav",
        "glasswing.m4a",
        "glasswing (lead-in).flac",
    ];
    #[cfg(windows)]
    {
        add("glasswing (320).mp3", audio::mp3(&cd, 320));
        add("glasswing (128).mp3", audio::mp3(&mono, 128));
        glasswing.extend(["glasswing (320).mp3", "glasswing (128).mp3"]);
    }

    // ---- Versions: cuts are slices of the extended mix --------------------
    let extended = audio::song(HARBOR, 140.0, RATE);
    let original = cut(&extended, 30.0, 110.0);
    let radio = cut(&original, 20.0, 60.0);
    add(EXTENDED, audio::flac(&pcm(&extended)));
    add(ORIGINAL, audio::flac(&pcm(&original)));
    #[cfg(windows)]
    add(RADIO, audio::mp3(&pcm(&radio), 320));
    #[cfg(not(windows))]
    add(RADIO, audio::wav(&pcm(&radio)));

    // A VIP: the first half of the original, then something else.
    let vip = [cut(&original, 0.0, 40.0), audio::song(203, 40.0, RATE)].concat();
    add(VIP, audio::wav(&pcm(&vip)));
    // A remix: another production that keeps a ten-second hook.
    let other = audio::song(204, 80.0, RATE);
    let remix = [
        cut(&other, 0.0, 30.0),
        cut(&original, 30.0, 40.0),
        cut(&other, 40.0, 80.0),
    ]
    .concat();
    add(REMIX, audio::wav(&pcm(&remix)));
    // A live take: the same song, played 2% faster.
    let mut live = pcm(&original);
    live.rate = RATE + RATE / 50;
    add(LIVE, audio::wav(&live));

    // ---- Clean and Dirty: three quarter-second mutes ----------------------
    let dirty = audio::song(MOTH, 60.0, RATE);
    let mut clean = dirty.clone();
    for at in [0.23, 0.51, 0.78] {
        let start = (at * clean.len() as f64) as usize;
        clean[start..start + RATE as usize / 4].fill(0.0);
    }
    add(DIRTY, audio::flac(&pcm(&dirty)));
    add(CLEAN, audio::flac(&pcm(&clean)));

    // A mashup: two songs on top of each other.
    let mashup: Vec<f64> = dirty
        .iter()
        .zip(&original)
        .map(|(a, b)| (a + b) / 2.0)
        .collect();
    add(MASHUP, audio::wav(&pcm(&mashup)));

    // ---- Same title, different songs --------------------------------------
    add("core (a).wav", audio::wav(&audio::pcm(406, 60.0, RATE, 1)));
    add("core (b).wav", audio::wav(&audio::pcm(407, 60.0, RATE, 1)));

    // ---- E3's shapes: what real cuts and reworks measured like -------------
    // An extended mix that isn't the original with more around it: a new
    // intro, and a breakdown put in two thirds of the way through. The
    // original lines up with it in two pieces at two shifts.
    let ridge = audio::song(501, 76.0, RATE);
    let ridge_extended = [
        audio::song(502, 18.0, RATE),
        cut(&ridge, 0.0, 55.0),
        audio::song(503, 16.0, RATE),
        cut(&ridge, 55.0, 76.0),
    ]
    .concat();
    add(RIDGE_ORIGINAL, audio::flac(&pcm(&ridge)));
    add(RIDGE_EXTENDED, audio::wav(&pcm(&ridge_extended)));

    // A radio edit that is one long stretch of the original, then jumps to
    // its ending.
    let quay = audio::song(511, 100.0, RATE);
    let quay_radio = [cut(&quay, 14.0, 62.0), cut(&quay, 84.0, 92.0)].concat();
    add(QUAY_ORIGINAL, audio::flac(&pcm(&quay)));
    #[cfg(windows)]
    add(QUAY_RADIO, audio::mp3(&pcm(&quay_radio), 192));
    #[cfg(not(windows))]
    add(QUAY_RADIO, audio::wav(&pcm(&quay_radio)));

    // A rework that keeps five four-second pieces of the original where
    // they were and replaces everything between them.
    let fen = audio::song(521, 90.0, RATE);
    let mut fen_rework = audio::song(522, 90.0, RATE);
    for piece in 0..5 {
        let from = ((8.0 + f64::from(piece) * 17.0) * f64::from(RATE)) as usize;
        let to = from + 4 * RATE as usize;
        fen_rework[from..to].copy_from_slice(&fen[from..to]);
    }
    add(FEN_ORIGINAL, audio::wav(&pcm(&fen)));
    add(FEN_REWORK, audio::wav(&pcm(&fen_rework)));

    // Another recording of a song under the same name (E3 pair #42): the
    // first 75 s follow the studio take, in another mix, and the rest is
    // something else.
    let ember = audio::song(531, 120.0, RATE);
    let ember_demo = [
        under(&cut(&ember, 0.0, 75.0), 532, 0.7),
        audio::song(533, 45.0, RATE),
    ]
    .concat();
    add(EMBER_STUDIO, audio::flac(&pcm(&ember)));
    add(EMBER_DEMO, audio::wav(&pcm(&ember_demo)));

    let harbor_family = [EXTENDED, ORIGINAL, RADIO, VIP, REMIX, LIVE];
    let mut other_reworks = vec![(RADIO, VIP), (DIRTY, MASHUP), (CLEAN, MASHUP)];
    for version in harbor_family {
        other_reworks.push((version, MASHUP));
        if version != REMIX && version != LIVE {
            other_reworks.push((version, REMIX));
            other_reworks.push((version, LIVE));
        }
    }
    other_reworks.push((REMIX, LIVE));

    Corpus {
        files,
        duplicate_groups: vec![glasswing],
        contains: vec![
            Contains {
                outer: EXTENDED,
                inner: ORIGINAL,
                at_seconds: 30.0,
            },
            Contains {
                outer: ORIGINAL,
                inner: RADIO,
                at_seconds: 20.0,
            },
            Contains {
                outer: EXTENDED,
                inner: RADIO,
                at_seconds: 50.0,
            },
        ],
        rough_cuts: vec![
            (RIDGE_ORIGINAL, RIDGE_EXTENDED),
            (QUAY_RADIO, QUAY_ORIGINAL),
        ],
        clean_dirty: vec![(DIRTY, CLEAN)],
        reworks_sharing_audio: vec![(ORIGINAL, VIP), (EXTENDED, VIP), (FEN_ORIGINAL, FEN_REWORK)],
        weak_matches: vec![(EMBER_STUDIO, EMBER_DEMO)],
        other_reworks,
    }
}
