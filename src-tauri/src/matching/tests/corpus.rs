//! A small made-up library with known answers, mirroring the duplicate and
//! version cases of `tools/fixture-gen`'s ground truth: one recording in
//! many formats, cuts that are exact slices of a longer mix, Clean and
//! Dirty, reworks that share a little or nothing, and unrelated songs.
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
    /// Cuts of the same length: the same production with a few words
    /// muted.
    pub clean_dirty: Vec<(&'static str, &'static str)>,
    /// Reworks sharing a good part of the audio (a VIP keeping the first
    /// half of the original).
    pub reworks_sharing_audio: Vec<(&'static str, &'static str)>,
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

    /// Every pair a fingerprint can tell is related: duplicates, slices,
    /// Clean/Dirty and the reworks sharing audio.
    pub fn true_pairs(&self) -> Vec<(&'static str, &'static str)> {
        let mut pairs = self.duplicate_pairs();
        pairs.extend(self.contains.iter().map(|c| (c.outer, c.inner)));
        pairs.extend(self.clean_dirty.iter().copied());
        pairs.extend(self.reworks_sharing_audio.iter().copied());
        pairs
    }

    /// Whether `a` and `b` are related in any way the corpus records.
    pub fn related(&self, a: &str, b: &str) -> bool {
        self.true_pairs()
            .into_iter()
            .chain(self.other_reworks.iter().copied())
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

const EXTENDED: &str = "harbor (extended).flac";
const ORIGINAL: &str = "harbor (original).flac";
const RADIO: &str = "harbor (radio edit)";
const VIP: &str = "harbor (vip).wav";
const REMIX: &str = "harbor (remix).wav";
const LIVE: &str = "harbor (live).wav";
const DIRTY: &str = "moth (dirty).flac";
const CLEAN: &str = "moth (clean).flac";
const MASHUP: &str = "harbor x moth (mashup).wav";

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
        clean_dirty: vec![(DIRTY, CLEAN)],
        reworks_sharing_audio: vec![(ORIGINAL, VIP), (EXTENDED, VIP)],
        other_reworks,
    }
}
