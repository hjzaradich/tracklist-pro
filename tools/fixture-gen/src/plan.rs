//! What to write: every file's path, content recipe and ground truth.
//!
//! The plan is built single-threaded from the seed, so it's the same every
//! run; the files are then rendered in any order, in parallel.
//!
//! All names are synthetic. Artist, title and album names are invented or
//! built from syllables; none are real releases.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use crate::encode::id3::Id3Version;
use crate::encode::Tags;
use crate::manifest::{
    Contains, DuplicateGroup, FileEntry, LinkKind, NotRelated, QualityCase, Recording, Role,
    SkipKind, Trap, TrapKind, VersionLink, MUSIC_ROOT,
};
use crate::rng::Rng;
use crate::sandbox::RelPath;
use crate::synth::{self, Pcm, Song, Timbre};

/// Core cases are 44.1 kHz, so the quality cases have a full spectrum.
pub const CORE_RATE: u32 = 44_100;
/// Bulk files are 22.05 kHz to stay small.
pub const BULK_RATE: u32 = 22_050;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp3 {
        kbps: u32,
        id3: Id3Version,
        v1: bool,
    },
    Flac,
    Wav,
    Aiff,
    M4a,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Mp3 { .. } => "mp3",
            Format::Flac => "flac",
            Format::Wav => "wav",
            Format::Aiff => "aiff",
            Format::M4a => "m4a",
        }
    }

    fn codec(self) -> &'static str {
        match self {
            Format::Mp3 { .. } => "mp3",
            Format::Flac => "flac",
            Format::Wav => "pcm_s16le",
            Format::Aiff => "pcm_s16be",
            Format::M4a => "alac",
        }
    }

    fn kbps(self) -> Option<u32> {
        match self {
            Format::Mp3 { kbps, .. } => Some(kbps),
            _ => None,
        }
    }
}

fn mp3(kbps: u32) -> Format {
    Format::Mp3 {
        kbps,
        id3: Id3Version::V23,
        v1: false,
    }
}

fn mp3_v24(kbps: u32) -> Format {
    Format::Mp3 {
        kbps,
        id3: Id3Version::V24,
        v1: false,
    }
}

/// Damage applied while rendering an audio file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    BadTdrc,
    BrokenApe,
    MissingMoov,
}

/// The `TDRC` value of the bad-date trap: month 13, day 45.
pub const BAD_TDRC: &str = "2019-13-45";

#[derive(Clone, Debug)]
pub enum Source {
    Pcm(Arc<Pcm>),
    /// A bulk tone, rendered when the file is written.
    Tone {
        seed: u64,
        rate: u32,
        ms: u32,
    },
}

impl Source {
    pub fn pcm(&self) -> Arc<Pcm> {
        match self {
            Source::Pcm(p) => p.clone(),
            &Source::Tone { seed, rate, ms } => {
                Arc::new(Pcm::from_float(rate, &synth::tone(seed, rate, ms)))
            }
        }
    }

    fn rate_and_frames(&self) -> (u32, u64) {
        match self {
            Source::Pcm(p) => (p.rate, p.frames()),
            &Source::Tone { rate, ms, .. } => (rate, (rate as u64 * ms as u64) / 1000),
        }
    }
}

// Audio is the common case; boxing it would only add an allocation per file.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Body {
    Audio {
        source: Source,
        format: Format,
        tags: Tags,
        damage: Option<Damage>,
    },
    Raw(Arc<Vec<u8>>),
}

#[derive(Clone, Debug)]
pub struct FileSpec {
    pub path: RelPath,
    pub body: Body,
    pub entry: FileEntry,
}

#[derive(Debug)]
pub struct Plan {
    pub files: Vec<FileSpec>,
    pub recordings: Vec<Recording>,
    pub version_links: Vec<VersionLink>,
    pub not_related: Vec<NotRelated>,
}

impl Plan {
    pub fn duplicate_groups(&self) -> Vec<DuplicateGroup> {
        self.recordings
            .iter()
            .filter_map(|r| {
                let files: Vec<String> = self
                    .files
                    .iter()
                    .filter(|f| f.entry.recording.as_deref() == Some(r.id.as_str()))
                    .map(|f| f.entry.path.clone())
                    .collect();
                (files.len() > 1).then(|| DuplicateGroup {
                    recording: r.id.clone(),
                    files,
                })
            })
            .collect()
    }

    /// Every folder that must exist, parents first.
    pub fn folders(&self) -> Vec<RelPath> {
        let mut set = BTreeSet::new();
        for f in &self.files {
            let mut p = f.path.parent();
            while let Some(dir) = p {
                p = dir.parent();
                set.insert(dir);
            }
        }
        let mut dirs: Vec<RelPath> = set.into_iter().collect();
        dirs.sort_by_key(|d| d.parts().len());
        dirs
    }
}

/// A recording the plan has rendered, for adding files of it.
#[derive(Clone)]
struct Rec {
    id: String,
    pcm: Arc<Pcm>,
}

struct Builder {
    seed: u64,
    files: Vec<FileSpec>,
    recordings: Vec<Recording>,
    links: Vec<VersionLink>,
    not_related: Vec<NotRelated>,
    /// Lowercased paths, since Windows names are case-insensitive.
    taken: HashSet<String>,
}

fn path(p: &str) -> RelPath {
    RelPath::parse(&format!("{MUSIC_ROOT}/{p}")).expect("plan paths are valid")
}

fn entry(path: &RelPath, role: Role, cases: &[&str]) -> FileEntry {
    FileEntry {
        path: path.to_manifest(),
        role,
        cases: cases.iter().map(|c| c.to_string()).collect(),
        recording: None,
        format: None,
        codec: None,
        bitrate_kbps: None,
        sample_rate: None,
        channels: None,
        frames: None,
        decodes: false,
        tags: None,
        byte_copy_of: None,
        trap: None,
        skip: None,
        quality: None,
        note: None,
        bytes: 0,
    }
}

impl Builder {
    fn new(seed: u64) -> Builder {
        Builder {
            seed,
            files: Vec::new(),
            recordings: Vec::new(),
            links: Vec::new(),
            not_related: Vec::new(),
            taken: HashSet::new(),
        }
    }

    /// A song seeded from the run's seed and a name.
    fn song(&self, name: &str) -> Song {
        Song::from_seed(Rng::derive(self.seed, name).next_u64(), CORE_RATE)
    }

    fn timbre(&self, name: &str) -> Timbre {
        Timbre::from_seed(Rng::derive(self.seed, name).next_u64())
    }

    fn recording(&mut self, id: &str, title: &str, artist: &str, audio: Vec<f64>) -> Rec {
        self.recording_with_note(id, title, artist, audio, None)
    }

    fn recording_with_note(
        &mut self,
        id: &str,
        title: &str,
        artist: &str,
        audio: Vec<f64>,
        note: Option<&str>,
    ) -> Rec {
        self.recordings.push(Recording {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            note: note.map(Into::into),
        });
        Rec {
            id: id.into(),
            pcm: Arc::new(Pcm::from_float(CORE_RATE, &audio)),
        }
    }

    fn link(&mut self, a: &Rec, b: &Rec, kind: LinkKind, label: &'static str) {
        self.link_with_note(a, b, kind, label, None);
    }

    fn link_with_note(
        &mut self,
        a: &Rec,
        b: &Rec,
        kind: LinkKind,
        label: &'static str,
        note: Option<&'static str>,
    ) {
        self.links.push(VersionLink {
            a: a.id.clone(),
            b: b.id.clone(),
            kind,
            label,
            note,
            contains: None,
        });
    }

    /// Records that `inner` is `outer`'s audio from `at_frame` on, on the
    /// link just added.
    fn contains(&mut self, outer: &Rec, inner: &Rec, at_frame: usize) {
        self.links
            .last_mut()
            .expect("a link was just added")
            .contains = Some(Contains {
            outer: outer.id.clone(),
            inner: inner.id.clone(),
            at_frame: at_frame as u64,
        });
    }

    fn push(&mut self, spec: FileSpec) -> usize {
        let key = spec.path.to_manifest().to_lowercase();
        assert!(
            self.taken.insert(key),
            "plan has two files at {}",
            spec.path
        );
        self.files.push(spec);
        self.files.len() - 1
    }

    fn audio_spec(
        path: RelPath,
        source: Source,
        format: Format,
        tags: Tags,
        cases: &[&str],
    ) -> FileSpec {
        let (rate, frames) = source.rate_and_frames();
        let mut e = entry(&path, Role::Audio, cases);
        e.format = Some(format.name());
        e.codec = Some(format.codec());
        e.bitrate_kbps = format.kbps();
        e.sample_rate = Some(rate);
        e.channels = Some(1);
        e.frames = Some(frames);
        e.decodes = true;
        e.tags = (!tags.is_empty()).then(|| tags.clone());
        FileSpec {
            path,
            body: Body::Audio {
                source,
                format,
                tags,
                damage: None,
            },
            entry: e,
        }
    }

    /// Adds a valid audio file of `rec`.
    fn audio(&mut self, p: &str, rec: &Rec, format: Format, tags: Tags, cases: &[&str]) -> usize {
        let mut spec = Self::audio_spec(path(p), Source::Pcm(rec.pcm.clone()), format, tags, cases);
        spec.entry.recording = Some(rec.id.clone());
        self.push(spec)
    }

    fn note(&mut self, index: usize, note: &str) {
        self.files[index].entry.note = Some(note.into());
    }

    /// A byte-for-byte copy of file `of` at another path.
    fn copy(&mut self, of: usize, p: &str, cases: &[&str]) -> usize {
        let src = self.files[of].clone();
        let path = path(p);
        let mut e = src.entry.clone();
        e.path = path.to_manifest();
        e.cases = cases.iter().map(|c| c.to_string()).collect();
        e.byte_copy_of = Some(src.entry.path.clone());
        e.note = None;
        self.push(FileSpec {
            path,
            body: src.body,
            entry: e,
        })
    }

    fn raw(&mut self, p: &str, bytes: Vec<u8>, mut e: FileEntry) -> usize {
        let path = path(p);
        e.path = path.to_manifest();
        self.push(FileSpec {
            path,
            body: Body::Raw(Arc::new(bytes)),
            entry: e,
        })
    }

    fn skip(&mut self, p: &str, kind: SkipKind) {
        let bytes = match kind {
            SkipKind::AppleDouble => apple_double(),
            SkipKind::DsStore => ds_store(),
        };
        let mut e = entry(&path(p), Role::Skip, &["skip"]);
        e.skip = Some(kind);
        self.raw(p, bytes, e);
    }
}

/// A minimal AppleDouble file: magic, version, filler, and one empty Finder
/// info entry, as macOS leaves on non-Mac drives.
pub fn apple_double() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0x0005_1607u32.to_be_bytes());
    b.extend_from_slice(&0x0002_0000u32.to_be_bytes());
    b.extend_from_slice(b"Mac OS X        ");
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&9u32.to_be_bytes()); // entry: Finder info
    b.extend_from_slice(&38u32.to_be_bytes()); // offset
    b.extend_from_slice(&32u32.to_be_bytes()); // length
    b.extend_from_slice(&[0; 32]);
    b
}

/// The start of a `.DS_Store` file (its "Bud1" buddy-allocator header).
pub fn ds_store() -> Vec<u8> {
    let mut b = vec![0, 0, 0, 1];
    b.extend_from_slice(b"Bud1");
    b.extend_from_slice(&[0, 0, 0x10, 0, 0, 0, 0x08, 0, 0, 0, 0x10, 0]);
    b.resize(64, 0);
    b
}

/// The ground-truth cases: duplicates, versions, awkward names, skips,
/// traps, a gig-stick copy and quality cases.
fn core(b: &mut Builder) {
    // ---- Duplicates: one recording, many files ----------------------------
    let glass_tags = || {
        Tags::new("Glasswing", "Nemora Vale")
            .album("Lanterns EP")
            .genre("Deep House")
            .year("2021")
    };
    let glasswing = {
        let song = b.song("glasswing");
        b.recording("glasswing", "Glasswing", "Nemora Vale", song.render(0..8))
    };
    let dup = ["duplicate"];
    let mut t = glass_tags();
    t.bpm = Some("122".into());
    t.key = Some("8A".into());
    b.audio(
        "Nemora Vale/Lanterns EP/01 Glasswing.flac",
        &glasswing,
        Format::Flac,
        t,
        &dup,
    );
    b.audio(
        "AIFF/Nemora Vale - Glasswing.aiff",
        &glasswing,
        Format::Aiff,
        glass_tags(),
        &dup,
    );
    b.audio(
        "WAV Masters/Glasswing.wav",
        &glasswing,
        Format::Wav,
        Tags::new("glasswing", "Nemora Vale"),
        &["duplicate", "different-tags"],
    );
    let glass_320 = b.audio(
        "Downloads/Nemora Vale - Glasswing (320).mp3",
        &glasswing,
        mp3(320),
        glass_tags(),
        &["duplicate", "bitrate"],
    );
    b.audio(
        "Downloads/Nemora Vale - Glasswing.mp3",
        &glasswing,
        mp3_v24(128),
        Tags::new("Glasswing", "Nemora Vale"),
        &["duplicate", "bitrate"],
    );
    b.audio(
        "iTunes/Nemora Vale/Glasswing.m4a",
        &glasswing,
        Format::M4a,
        glass_tags(),
        &dup,
    );
    b.audio(
        "Downloads/RipSite.example - Nemora Vale - Glasswing.mp3",
        &glasswing,
        mp3(128),
        Tags::new("Glasswing", "Nemora Vale"),
        &["duplicate", "rip-junk-name"],
    );
    let retag = b.audio(
        "Retagged/Glasswing (retag).mp3",
        &glasswing,
        mp3(320),
        Tags::new("Glasswing", "Nemora Vale")
            .genre("Tech House")
            .year("2020"),
        &["duplicate", "different-tags", "same-audio-stream"],
    );
    b.note(
        retag,
        "Same MP3 audio frames as 'Downloads/Nemora Vale - Glasswing (320).mp3'; only the ID3 tag differs.",
    );

    let orbit = {
        let song = b.song("orbit-line");
        b.recording(
            "orbit-line",
            "Orbit Line (Original Mix)",
            "Halden Rook",
            song.render(0..8),
        )
    };
    let orbit_tags = || Tags::new("Orbit Line (Original Mix)", "Halden Rook").genre("Techno");
    let orbit_bp = b.audio(
        "Beatport/10000001_Orbit_Line_&#40;Original Mix&#41;.mp3",
        &orbit,
        mp3(320),
        orbit_tags(),
        &["duplicate", "store-id-prefix", "html-entities"],
    );
    b.audio(
        "Halden Rook/Orbit Line (Original Mix).flac",
        &orbit,
        Format::Flac,
        orbit_tags(),
        &dup,
    );
    b.audio(
        "Downloads/Orbit Line [ ripper.example ].mp3",
        &orbit,
        mp3(128),
        Tags::default(),
        &["duplicate", "rip-junk-name", "untagged"],
    );
    // A different rip of the same recording: a little extra silence first.
    let lead_in = {
        let mut audio = vec![0.0; (CORE_RATE / 5) as usize];
        audio.extend(b.song("orbit-line").render(0..8));
        let pcm = Arc::new(Pcm::from_float(CORE_RATE, &audio));
        let spec = Builder::audio_spec(
            path("Downloads/Halden Rook - Orbit Line (Original Mix) [www.ripper.example].mp3"),
            Source::Pcm(pcm),
            mp3(192),
            orbit_tags(),
            &["duplicate", "rip-junk-name", "lead-in-silence"],
        );
        b.push(spec)
    };
    b.files[lead_in].entry.recording = Some(orbit.id.clone());
    b.note(
        lead_in,
        "Same recording, different rip: 0.2 s of extra silence at the start.",
    );

    let pulse = {
        let song = b.song("untitled-pulse");
        b.recording("untitled-pulse", "", "", song.render(0..6))
    };
    b.audio(
        "Unsorted/Track 01.wav",
        &pulse,
        Format::Wav,
        Tags::default(),
        &["duplicate", "untagged"],
    );
    let pulse_mp3 = b.audio(
        "Unsorted/track01.mp3",
        &pulse,
        mp3(192),
        Tags::default(),
        &["duplicate", "untagged"],
    );

    // ---- Versions: linked, never merged -----------------------------------
    let harbor = b.song("paper-harbor");
    let harbor_tags = || {
        Tags::new("Paper Harbor (Original Mix)", "Kestrel Nine")
            .album("Paper Harbor")
            .genre("Progressive House")
            .year("2019")
    };
    let original = b.recording(
        "paper-harbor-original",
        "Paper Harbor (Original Mix)",
        "Kestrel Nine",
        harbor.render(0..8),
    );
    b.audio(
        "Kestrel Nine/Paper Harbor/Paper Harbor (Original Mix).flac",
        &original,
        Format::Flac,
        harbor_tags(),
        &["version"],
    );

    let extended = b.recording(
        "paper-harbor-extended",
        "Paper Harbor (Extended Mix)",
        "Kestrel Nine",
        harbor.render(-3..11),
    );
    b.audio(
        "Kestrel Nine/Paper Harbor/Paper Harbor (Extended Mix).flac",
        &extended,
        Format::Flac,
        Tags::new("Paper Harbor (Extended Mix)", "Kestrel Nine").album("Paper Harbor"),
        &["version", "cut"],
    );
    b.link_with_note(
        &original,
        &extended,
        LinkKind::Cut,
        "extended",
        Some("The whole original sits inside the extended mix (one-sided coverage)."),
    );
    b.contains(&extended, &original, 3 * harbor.bar_len());

    let radio = b.recording(
        "paper-harbor-radio",
        "Paper Harbor (Radio Edit)",
        "Kestrel Nine",
        harbor.render(2..6),
    );
    let radio_file = b.audio(
        "Kestrel Nine/Paper Harbor/Paper Harbor (Radio Edit).mp3",
        &radio,
        mp3(320),
        Tags::new("Paper Harbor (Radio Edit)", "Kestrel Nine").album("Paper Harbor"),
        &["version", "cut"],
    );
    b.link_with_note(
        &original,
        &radio,
        LinkKind::Cut,
        "radio edit",
        Some("The radio edit is a slice of the original."),
    );
    b.contains(&original, &radio, 2 * harbor.bar_len());

    // Reworks keep a little of the original (a hook) and replace the rest,
    // as real ones do: E3 found 0–25% fingerprint coverage for most.
    let rework = |seed_xor: u64, timbre: Timbre, keep: std::ops::Range<i32>, len: i32| {
        let mut new = harbor.clone();
        new.melody_seed ^= seed_xor;
        new.timbre = timbre;
        let mut hook = harbor.clone();
        hook.timbre = timbre;
        (0..len)
            .flat_map(|i| {
                if keep.contains(&i) {
                    hook.bar(i)
                } else {
                    new.bar(i)
                }
            })
            .collect::<Vec<f64>>()
    };
    let remix = {
        let audio = rework(0x4e3, b.timbre("quill-remix"), 3..5, 8);
        b.recording(
            "paper-harbor-quill-remix",
            "Paper Harbor (Quill Ashby Remix)",
            "Quill Ashby",
            audio,
        )
    };
    b.audio(
        "Quill Ashby/Paper Harbor (Quill Ashby Remix).mp3",
        &remix,
        mp3(320),
        Tags::new("Paper Harbor (Quill Ashby Remix)", "Quill Ashby"),
        &["version", "rework", "remixer-only-credit"],
    );
    b.link_with_note(
        &original,
        &remix,
        LinkKind::Rework,
        "remix",
        Some("Credited only to the remixer; the original artist isn't in the tags."),
    );

    let vip = {
        let mut other = harbor.clone();
        other.melody_seed ^= 0x5555;
        let mut audio = harbor.render(0..4);
        audio.extend(other.render(4..8));
        b.recording(
            "paper-harbor-vip",
            "Paper Harbor (VIP)",
            "Kestrel Nine",
            audio,
        )
    };
    b.audio(
        "Kestrel Nine/Paper Harbor (VIP).wav",
        &vip,
        Format::Wav,
        Tags::new("Paper Harbor (VIP)", "Kestrel Nine"),
        &["version", "rework"],
    );
    b.link_with_note(
        &original,
        &vip,
        LinkKind::Rework,
        "vip",
        Some("Shares its first half with the original, then differs."),
    );

    let bootleg = {
        let mut timbre = harbor.timbre;
        timbre.drum_seed ^= 0xb007;
        timbre.hiss = 0.01;
        let audio = rework(0xb007, timbre, 0..2, 8);
        b.recording(
            "paper-harbor-bootleg",
            "Paper Harbor (@lumenlark edit)",
            "Kestrel Nine",
            audio,
        )
    };
    b.audio(
        "Edits/Paper Harbor (@lumenlark edit).mp3",
        &bootleg,
        mp3(320),
        Tags::new("Paper Harbor (@lumenlark edit)", "Kestrel Nine"),
        &["version", "rework", "same-length"],
    );
    b.link_with_note(
        &original,
        &bootleg,
        LinkKind::Rework,
        "bootleg",
        Some("Exactly the original's length, different audio: length decides nothing."),
    );

    let flip = {
        let audio = rework(0xf11f, b.timbre("vey-flip"), 5..6, 7);
        b.recording(
            "paper-harbor-flip",
            "Paper Harbor [Vey Sun Flip]",
            "Kestrel Nine",
            audio,
        )
    };
    b.audio(
        "Edits/Paper Harbor [Vey Sun Flip].mp3",
        &flip,
        mp3(256),
        Tags::new("Paper Harbor [Vey Sun Flip]", "Kestrel Nine"),
        &["version", "rework"],
    );
    b.link(&original, &flip, LinkKind::Rework, "flip");

    let cover = {
        let mut s = harbor.clone();
        s.timbre = b.timbre("marrow-cover");
        s.bar_ms += 150;
        b.recording(
            "paper-harbor-cover",
            "Paper Harbor",
            "The Marrow Choir",
            s.render(0..8),
        )
    };
    b.audio(
        "The Marrow Choir/Paper Harbor.m4a",
        &cover,
        Format::M4a,
        Tags::new("Paper Harbor", "The Marrow Choir"),
        &["version", "rework", "cover"],
    );
    b.link_with_note(
        &original,
        &cover,
        LinkKind::Rework,
        "cover",
        Some("Another artist's recording of the same song; shown with a \"cover\" label."),
    );

    let live = {
        // Same song, played a little slower, with a different kit and crowd
        // noise: it drifts out of step with the studio take.
        let mut s = harbor.clone();
        s.bar_ms += 60;
        s.timbre.drum_seed ^= 0x11fe;
        s.timbre.hiss = 0.04;
        s.timbre.harmonics[0] *= 0.5;
        b.recording_with_note(
            "paper-harbor-live",
            "Paper Harbor (Original Mix)",
            "Kestrel Nine",
            s.render(0..8),
            Some("A live recording tagged exactly like the studio original."),
        )
    };
    let live_file = b.audio(
        "Kestrel Nine/Live/Paper Harbor (Original Mix).mp3",
        &live,
        mp3(320),
        harbor_tags(),
        &["version", "rework", "identical-tags"],
    );
    b.note(
        live_file,
        "Tags identical to the original's; only the audio tells them apart.",
    );
    b.link_with_note(
        &original,
        &live,
        LinkKind::Rework,
        "live",
        Some("Identical tags hide a different recording: the fingerprint must veto a name-based merge."),
    );

    // Clean and Dirty: same production, a few words muted.
    let moth = b.song("neon-moth");
    let bleeps = |len: usize| -> Vec<std::ops::Range<usize>> {
        [0.23, 0.51, 0.78]
            .iter()
            .map(|&at| {
                let s = (at * len as f64) as usize;
                s..s + CORE_RATE as usize / 4
            })
            .collect()
    };
    let dirty_audio = moth.render(0..8);
    let clean_audio = synth::mute(dirty_audio.clone(), &bleeps(dirty_audio.len()));
    let dirty = b.recording(
        "neon-moth-dirty",
        "Neon Moth (Dirty)",
        "Solvane",
        dirty_audio,
    );
    let clean = b.recording(
        "neon-moth-clean",
        "Neon Moth (Clean)",
        "Solvane",
        clean_audio,
    );
    let dirty_file = b.audio(
        "Solvane/Neon Moth (Dirty).mp3",
        &dirty,
        mp3(320),
        Tags::new("Neon Moth (Dirty)", "Solvane"),
        &["version", "cut"],
    );
    b.audio(
        "Solvane/Neon Moth (Clean).mp3",
        &clean,
        mp3(320),
        Tags::new("Neon Moth (Clean)", "Solvane"),
        &["version", "cut"],
    );
    b.link_with_note(
        &dirty,
        &clean,
        LinkKind::Cut,
        "clean/dirty",
        Some("Identical length and nearly identical audio (three 0.25 s mutes): only the names tell them apart."),
    );

    // A version word that's part of the real title.
    let velo = b.song("velo");
    let velo_dirty_audio = velo.render(0..6);
    let velo_clean_audio = synth::mute(velo_dirty_audio.clone(), &bleeps(velo_dirty_audio.len()));
    let velo_title = b.recording_with_note(
        "velo-clean",
        "Velo (dirty)",
        "Tovi Ash",
        velo_clean_audio,
        Some("\"(dirty)\" is part of the real title; this file is the clean cut."),
    );
    let velo_dirty = b.recording(
        "velo-dirty",
        "Velo (dirty) (dirty)",
        "Tovi Ash",
        velo_dirty_audio,
    );
    b.audio(
        "Tovi Ash/Velo (dirty).mp3",
        &velo_title,
        mp3(256),
        Tags::new("Velo (dirty)", "Tovi Ash"),
        &["version", "cut", "version-word-in-title"],
    );
    b.audio(
        "Tovi Ash/Velo (dirty) (dirty).mp3",
        &velo_dirty,
        mp3(256),
        Tags::new("Velo (dirty) (dirty)", "Tovi Ash"),
        &["version", "cut", "version-word-in-title"],
    );
    b.link_with_note(
        &velo_title,
        &velo_dirty,
        LinkKind::Cut,
        "clean/dirty",
        Some("Never strip version words blindly: \"Velo (dirty)\" is the title."),
    );

    // A mashup, linked to both sources.
    let mashup = {
        let audio = synth::mix(&harbor.render(0..8), &moth.render(0..8));
        b.recording(
            "harbor-x-moth",
            "Paper Harbor x Neon Moth (Vey Sun Mashup)",
            "Vey Sun",
            audio,
        )
    };
    b.audio(
        "Edits/Paper Harbor x Neon Moth (Vey Sun Mashup).mp3",
        &mashup,
        mp3(320),
        Tags::new("Paper Harbor x Neon Moth (Vey Sun Mashup)", "Vey Sun"),
        &["version", "rework", "mashup"],
    );
    b.link(&original, &mashup, LinkKind::Rework, "mashup");
    b.link(&dirty, &mashup, LinkKind::Rework, "mashup");

    // Names that look related but aren't.
    let core_a = {
        let s = b.song("core-a");
        b.recording(
            "korvex-rivetta-remix",
            "Korvex (Rivetta Remix)",
            "Rivetta",
            s.render(0..8),
        )
    };
    let core_b = {
        let s = b.song("core-b");
        b.recording(
            "korvex-flint-bootleg",
            "Korvex (Flint Bootleg)",
            "Flint Harlow",
            s.render(0..8),
        )
    };
    b.audio(
        "Edits/Korvex (Rivetta Remix).mp3",
        &core_a,
        mp3(320),
        Tags::new("Korvex (Rivetta Remix)", "Rivetta"),
        &["not-related", "same-title"],
    );
    b.audio(
        "Edits/Korvex (Flint Bootleg).mp3",
        &core_b,
        mp3(320),
        Tags::new("Korvex (Flint Bootleg)", "Flint Harlow"),
        &["not-related", "same-title"],
    );
    b.not_related.push(NotRelated {
        a: core_a.id.clone(),
        b: core_b.id.clone(),
        note: "Same title, different original songs.",
    });

    let dusk = {
        let s = b.song("dusk-palo");
        b.recording("duskline", "Duskline", "Palo Mirren", s.render(0..8))
    };
    let dusk_mashup = {
        let audio = synth::mix(
            &b.song("dusk-ilse").render(0..8),
            &b.song("ember-ilse").render(0..8),
        );
        b.recording(
            "duskline-x-emberlow",
            "Duskline x Emberlow (Mashup)",
            "Ilse Varn",
            audio,
        )
    };
    b.audio(
        "Palo Mirren/Duskline.flac",
        &dusk,
        Format::Flac,
        Tags::new("Duskline", "Palo Mirren"),
        &["not-related"],
    );
    b.audio(
        "Edits/Duskline x Emberlow (Mashup).mp3",
        &dusk_mashup,
        mp3(320),
        Tags::new("Duskline x Emberlow (Mashup)", "Ilse Varn"),
        &["not-related", "mashup"],
    );
    b.not_related.push(NotRelated {
        a: dusk.id.clone(),
        b: dusk_mashup.id.clone(),
        note:
            "The mashup's \"Duskline\" is a different song: part names can match unrelated tracks.",
    });

    // ---- Awkward names -----------------------------------------------------
    let awkward =
        |b: &mut Builder, id: &str, p: &str, format: Format, case: &str, note: Option<&str>| {
            let title = p.rsplit('/').next().unwrap();
            let title = title.rsplit_once('.').map_or(title, |(t, _)| t).to_owned();
            let song = b.song(id);
            let rec = b.recording(id, &title, "Name Test", song.render(0..3));
            let i = b.audio(
                p,
                &rec,
                format,
                Tags::new(&title, "Name Test"),
                &["awkward-name", case],
            );
            if let Some(n) = note {
                b.note(i, n);
            }
        };
    awkward(
        b,
        "luma-dot",
        "Q.V.X./Monolith/Luma.mp3",
        mp3(192),
        "awkward-name:trailing-dot",
        Some("Folder name ends in a dot. A plain Windows path drops it and opens the sibling 'Q.V.X' instead."),
    );
    awkward(
        b,
        "luma-nodot",
        "Q.V.X/Monolith/Luma.mp3",
        mp3(192),
        "awkward-name:trailing-dot-sibling",
        None,
    );
    awkward(
        b,
        "halvane-space",
        "Drift Unit /Halvane.flac",
        Format::Flac,
        "awkward-name:trailing-space",
        Some("Folder name ends in a space; the sibling 'Drift Unit' holds a different file."),
    );
    awkward(
        b,
        "halvane-nospace",
        "Drift Unit/Halvane.flac",
        Format::Flac,
        "awkward-name:trailing-space-sibling",
        None,
    );
    awkward(
        b,
        "con",
        "Misc/CON.mp3",
        mp3(128),
        "awkward-name:reserved",
        None,
    );
    awkward(
        b,
        "aux",
        "Misc/AUX.mp3",
        mp3(128),
        "awkward-name:reserved",
        None,
    );
    awkward(
        b,
        "vireo-nfc",
        "Misc/Caf\u{e9} Vireo.mp3",
        mp3(128),
        "awkward-name:nfc",
        Some("NFC twin of the NFD name next to it: different files, same NFC match key."),
    );
    awkward(
        b,
        "vireo-nfd",
        "Misc/Cafe\u{301} Vireo.mp3",
        mp3(128),
        "awkward-name:nfd",
        Some("NFD twin of the NFC name next to it: different files, same NFC match key."),
    );
    awkward(
        b,
        "ignis",
        "\u{2728} Stardust/\u{1f525} Ignis \u{1f525}.mp3",
        mp3(160),
        "awkward-name:emoji",
        None,
    );
    awkward(
        b,
        "vox-lumen",
        "\u{2728} Stardust/\u{1f469}\u{200d}\u{1f3a4} Vox Lumen.flac",
        Format::Flac,
        "awkward-name:emoji",
        None,
    );
    awkward(
        b,
        "cjk-ja",
        "青い回路/夜の信号.flac",
        Format::Flac,
        "awkward-name:cjk",
        None,
    );
    awkward(
        b,
        "cjk-ko",
        "소리 공장/새벽 회로.mp3",
        mp3(192),
        "awkward-name:cjk",
        None,
    );
    awkward(
        b,
        "specials",
        "#1 Hits & 100% Mixes/Nox & Vey #2 (100% Tape) + 'Edit'.mp3",
        mp3(192),
        "awkward-name:specials",
        Some("Holds # % + & ' in folder and file names; rekordbox writes # raw in Location."),
    );
    awkward(
        b,
        "late-night",
        "#1 Hits & 100% Mixes/Late%20Night.mp3",
        mp3(192),
        "awkward-name:percent-escape",
        Some("A literal \"%20\" in the name: decoding a Location must not turn it into a space."),
    );
    awkward(
        b,
        "lofi-run",
        "#1 Hits & 100% Mixes/Lo+Fi Run, Pt. (2).wav",
        Format::Wav,
        "awkward-name:specials",
        None,
    );
    awkward(
        b,
        "shout",
        "Misc/SHOUT.MP3",
        mp3(128),
        "awkward-name:uppercase-extension",
        None,
    );
    awkward(
        b,
        "quiet-room",
        "Misc/Quiet Room.Flac",
        Format::Flac,
        "awkward-name:mixed-case-extension",
        None,
    );
    let long_dir = "Extremely Long Folder Name For Testing Paths That Pass The Old Windows Limit";
    awkward(
        b,
        "long-path",
        &format!("{long_dir} 1/{long_dir} 2/{long_dir} 3/Deep Inside A Very Long Path Beyond MAX_PATH.aiff"),
        Format::Aiff,
        "awkward-name:long-path",
        Some("The path passes 260 characters even before the target folder's own path."),
    );

    // ---- Skip cases -------------------------------------------------------
    b.skip("Q.V.X/Monolith/._Luma.mp3", SkipKind::AppleDouble);
    b.skip(
        "Nemora Vale/Lanterns EP/._01 Glasswing.flac",
        SkipKind::AppleDouble,
    );
    b.skip(".DS_Store", SkipKind::DsStore);
    b.skip("Nemora Vale/.DS_Store", SkipKind::DsStore);

    // ---- Traps -------------------------------------------------------------
    let sable = {
        let s = b.song("sable-run");
        b.recording("sable-run", "Sable Run", "Orrin Vale", s.render(0..4))
    };
    let i = b.audio(
        "Downloads/Sable Run.wav",
        &sable,
        mp3(192),
        Tags::new("Sable Run", "Orrin Vale"),
        &["trap"],
    );
    b.files[i].entry.role = Role::Trap;
    b.files[i].entry.trap = Some(Trap {
        kind: TrapKind::Mp3InWav,
        detail:
            "Named .wav, but the bytes are an ID3-tagged MP3. Detect the format from the bytes.",
    });

    let unfinished = {
        let pcm = Arc::new(Pcm::from_float(
            CORE_RATE,
            &b.song("unfinished").render(0..4),
        ));
        let p = path("Downloads/Unfinished Download.m4a");
        let mut e = entry(&p, Role::Trap, &["trap"]);
        e.format = Some("m4a");
        e.trap = Some(Trap {
            kind: TrapKind::M4aNoMoov,
            detail: "ftyp and mdat, but no moov box: an interrupted download. Flag as broken.",
        });
        FileSpec {
            path: p,
            body: Body::Audio {
                source: Source::Pcm(pcm),
                format: Format::M4a,
                tags: Tags::default(),
                damage: Some(Damage::MissingMoov),
            },
            entry: e,
        }
    };
    b.push(unfinished);

    let damaged = |b: &mut Builder, id: &str, p: &str, damage: Damage, trap: Trap| {
        let song = b.song(id);
        let title = p
            .rsplit('/')
            .next()
            .unwrap()
            .trim_end_matches(".mp3")
            .to_owned();
        let rec = b.recording(id, &title, "Tag Test", song.render(0..3));
        let format = if damage == Damage::BadTdrc {
            mp3_v24(192)
        } else {
            mp3(192)
        };
        let i = b.audio(p, &rec, format, Tags::new(&title, "Tag Test"), &["trap"]);
        let f = &mut b.files[i];
        f.entry.role = Role::Trap;
        f.entry.trap = Some(trap);
        if let Body::Audio { damage: d, .. } = &mut f.body {
            *d = Some(damage);
        }
    };
    damaged(
        b,
        "bad-date",
        "Downloads/Bad Date.mp3",
        Damage::BadTdrc,
        Trap {
            kind: TrapKind::BadTdrc,
            detail:
                "ID3v2.4 TDRC is \"2019-13-45\". Read the other tags and index the audio anyway.",
        },
    );
    damaged(
        b,
        "broken-ape",
        "Downloads/Broken Ape.mp3",
        Damage::BrokenApe,
        Trap {
            kind: TrapKind::BrokenApe,
            detail: "An APEv2 footer claims a 16 MB tag and 7 items; only one item is there. Index the audio anyway.",
        },
    );

    let mut e = entry(&path("Downloads/Zero Bytes.mp3"), Role::Trap, &["trap"]);
    e.trap = Some(Trap {
        kind: TrapKind::EmptyFile,
        detail: "An empty file from a failed download. Flag as broken.",
    });
    b.raw("Downloads/Zero Bytes.mp3", Vec::new(), e);

    // ---- A gig-stick copy (rekordbox's Contents\Artist\Album\ layout) -----
    let gig = ["duplicate", "gig-stick-copy"];
    b.copy(
        glass_320,
        "Old USB Backup/Contents/Nemora Vale/Lanterns EP/Nemora Vale - Glasswing (320).mp3",
        &gig,
    );
    b.copy(
        radio_file,
        "Old USB Backup/Contents/Kestrel Nine/Paper Harbor/Paper Harbor (Radio Edit).mp3",
        &gig,
    );
    b.copy(
        dirty_file,
        "Old USB Backup/Contents/Solvane/UnknownAlbum/Neon Moth (Dirty).mp3",
        &gig,
    );
    b.copy(
        pulse_mp3,
        "Old USB Backup/Contents/UnknownArtist/UnknownAlbum/track01.mp3",
        &gig,
    );
    b.copy(
        orbit_bp,
        "Old USB Backup/Contents/Halden Rook/UnknownAlbum/10000001_Orbit_Line_&#40;Original Mix&#41;.mp3",
        &gig,
    );

    // ---- Quality (ROADMAP 1.6) --------------------------------------------
    let quality =
        |b: &mut Builder, id: &str, p: &str, format: Format, max_hz: u32, expect: &'static str| {
            let seed = Rng::derive(b.seed, id).next_u64();
            let audio = synth::band_limited(seed, CORE_RATE, 3000, f64::from(max_hz));
            let title = p
                .rsplit('/')
                .next()
                .unwrap()
                .rsplit_once('.')
                .unwrap()
                .0
                .to_owned();
            let rec = b.recording(id, &title, "Quality Test", audio);
            let i = b.audio(
                p,
                &rec,
                format,
                Tags::new(&title, "Quality Test"),
                &["quality"],
            );
            b.files[i].entry.quality = Some(QualityCase {
                expect,
                content_max_hz: max_hz,
            });
        };
    quality(
        b,
        "q-fake-320",
        "Quality Check/Fake 320.mp3",
        mp3(320),
        16_000,
        "suspect_transcode",
    );
    quality(
        b,
        "q-real-320",
        "Quality Check/Real 320.mp3",
        mp3(320),
        21_000,
        "ok",
    );
    quality(
        b,
        "q-flac-from-mp3",
        "Quality Check/FLAC From MP3.flac",
        Format::Flac,
        16_000,
        "suspect_transcode",
    );
    quality(
        b,
        "q-real-flac",
        "Quality Check/Real FLAC.flac",
        Format::Flac,
        21_000,
        "ok",
    );
}

const SYLLABLES: &[&str] = &[
    "ka", "lo", "vi", "ren", "tas", "mo", "zel", "qui", "dar", "nex", "sol", "bri", "vo", "lun",
    "fa", "rix", "tem", "ul", "sar", "po", "gen", "shi", "var", "no", "cal", "dri", "ex", "mau",
    "ter", "yon", "zi", "pel",
];

const GENRES: &[&str] = &[
    "House",
    "Deep House",
    "Tech House",
    "Techno",
    "Drum & Bass",
    "Dubstep",
    "Trap",
    "Hip-Hop",
    "Disco",
    "UK Garage",
    "Breaks",
    "Afro House",
    "Pop",
    "R&B",
    "Amapiano",
    "Jersey Club",
];

fn word(rng: &mut Rng) -> String {
    let n = rng.range(2, 3);
    let w: String = (0..n).map(|_| *rng.pick(SYLLABLES)).collect();
    let mut c = w.chars();
    let first = c.next().unwrap().to_uppercase();
    first.chain(c).collect()
}

fn words(rng: &mut Rng, lo: u64, hi: u64) -> String {
    (0..rng.range(lo, hi))
        .map(|_| word(rng))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bulk files: distinct audio with plausible synthetic tags, spread over a
/// Genre/Artist/Album tree and one big flat download folder.
fn bulk(b: &mut Builder, count: usize, ms: u32) {
    let mut names = Rng::derive(b.seed, "bulk names");
    let artists: Vec<(String, String, [String; 3])> = (0..(count / 15).max(8))
        .map(|_| {
            let artist = words(&mut names, 1, 2);
            let genre = names.pick(GENRES).to_string();
            let albums = [
                words(&mut names, 1, 2),
                format!("{} EP", words(&mut names, 1, 2)),
                words(&mut names, 1, 3),
            ];
            (artist, genre, albums)
        })
        .collect();
    let mut track_no = std::collections::HashMap::<(usize, usize), u32>::new();

    for i in 0..count {
        let mut rng = Rng::derive(b.seed, &format!("bulk {i}"));
        let a = rng.below(artists.len() as u64) as usize;
        let (artist, genre, albums) = &artists[a];
        let al = rng.below(3) as usize;
        let title = words(&mut rng, 1, 3);
        let format = match rng.unit() {
            u if u < 0.62 => Format::Mp3 {
                kbps: *rng.pick(&[64, 96, 128, 160]),
                id3: if rng.chance(0.5) {
                    Id3Version::V23
                } else {
                    Id3Version::V24
                },
                v1: rng.chance(0.2),
            },
            u if u < 0.74 => Format::Flac,
            u if u < 0.83 => Format::Wav,
            u if u < 0.91 => Format::Aiff,
            _ => Format::M4a,
        };
        let n = track_no.entry((a, al)).or_insert(0);
        *n += 1;
        let tags = if rng.chance(0.92) {
            Tags {
                title: Some(title.clone()),
                artist: Some(artist.clone()),
                album: Some(albums[al].clone()),
                genre: Some(genre.clone()),
                year: Some(rng.range(1990, 2026).to_string()),
                track: Some(n.to_string()),
                bpm: Some(rng.range(70, 174).to_string()),
                key: Some(format!(
                    "{}{}",
                    rng.range(1, 12),
                    if rng.chance(0.5) { "A" } else { "B" }
                )),
            }
        } else {
            Tags::default()
        };
        let ext = format.name();
        let base = if rng.chance(0.1) {
            format!("Downloads/Bulk/{artist} - {title}")
        } else {
            format!("Library/{genre}/{artist}/{}/{n:02} {title}", albums[al])
        };
        let mut p = format!("{base}.{ext}");
        let lower = |p: &str| format!("{MUSIC_ROOT}/{p}").to_lowercase();
        if b.taken.contains(&lower(&p)) {
            p = format!("{base} ({i}).{ext}");
        }
        let source = Source::Tone {
            seed: rng.next_u64(),
            rate: BULK_RATE,
            ms,
        };
        let mut spec = Builder::audio_spec(path(&p), source, format, tags, &["bulk"]);
        spec.entry.recording = Some(format!("bulk-{i:06}"));
        b.push(spec);
    }
}

/// Plans `total` files: the core cases, then bulk files up to `total`.
/// Bulk recordings aren't listed in `recordings` (each is one file).
pub fn plan(seed: u64, total: usize, bulk_ms: u32) -> Plan {
    let mut b = Builder::new(seed);
    core(&mut b);
    let n = total.saturating_sub(b.files.len());
    bulk(&mut b, n, bulk_ms);
    Plan {
        files: b.files,
        recordings: b.recordings,
        version_links: b.links,
        not_related: b.not_related,
    }
}
