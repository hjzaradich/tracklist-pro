//! Dev-only synthetic fixture generator (TASKS 0D-7).
//!
//! Writes a folder of small, valid audio files with synthetic tags, plus a
//! JSON manifest of the ground truth (duplicates, versions, awkward names,
//! skips, traps). The same seed always gives the same bytes. It writes only
//! inside the folder it's given; see [`sandbox`].
//!
//! Not part of the app: nothing in `src-tauri` depends on this package.

pub mod encode;
pub mod manifest;
pub mod plan;
pub mod rng;
pub mod sandbox;
pub mod synth;

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use encode::id3::{self, Id3Version};
use encode::{aiff, ape, flac, m4a, mp3, wav};
use manifest::{Counts, Manifest, Role};
use plan::{Body, Damage, Format, Plan};
use sandbox::{RelPath, SandboxError, Target};

#[derive(Clone, Debug)]
pub struct Options {
    pub seed: u64,
    /// Total files, core cases included. Below the core size, you get the
    /// core cases alone.
    pub files: usize,
    /// Length of each bulk file.
    pub bulk_ms: u32,
    /// Worker threads; 0 means one per core.
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: 1,
            files: 0,
            bulk_ms: 500,
            threads: 0,
        }
    }
}

#[derive(Debug)]
pub struct Summary {
    pub files: usize,
    pub bytes: u64,
    pub elapsed: Duration,
}

/// The bytes of one planned file.
pub fn render(body: &Body) -> Vec<u8> {
    let (source, format, tags, damage) = match body {
        Body::Raw(bytes) => return bytes.to_vec(),
        Body::Audio {
            source,
            format,
            tags,
            damage,
        } => (source, *format, tags, *damage),
    };
    let pcm = source.pcm();
    match format {
        Format::Mp3 { kbps, id3: ver, v1 } => {
            let tag = if damage == Some(Damage::BadTdrc) {
                let title = tags.title.as_deref().unwrap_or("");
                let artist = tags.artist.as_deref().unwrap_or("");
                id3::tag_from_frames(
                    &[
                        (b"TIT2", title),
                        (b"TPE1", artist),
                        (b"TDRC", plan::BAD_TDRC),
                    ],
                    Id3Version::V24,
                )
            } else {
                id3::tag(tags, ver)
            };
            let mut out = tag;
            out.extend_from_slice(&mp3::encode(&pcm, kbps));
            if damage == Some(Damage::BrokenApe) {
                out.extend_from_slice(&ape::broken(tags.title.as_deref().unwrap_or("")));
            }
            if v1 && !tags.is_empty() {
                out.extend_from_slice(&id3::v1(tags));
            }
            out
        }
        Format::Flac => flac::encode(&pcm, tags),
        Format::Wav => wav::encode(&pcm, tags),
        Format::Aiff => aiff::encode(&pcm, tags),
        Format::M4a if damage == Some(Damage::MissingMoov) => m4a::encode_without_moov(&pcm),
        Format::M4a => m4a::encode(&pcm, tags),
    }
}

#[derive(Debug)]
pub enum Error {
    Sandbox(SandboxError),
    Json(serde_json::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sandbox(e) => e.fmt(f),
            Error::Json(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<SandboxError> for Error {
    fn from(e: SandboxError) -> Self {
        Error::Sandbox(e)
    }
}

/// Writes the fixture tree and manifest into `target`, which must be new or
/// empty.
pub fn generate(target: &Path, opts: &Options) -> Result<Summary, Error> {
    let start = Instant::now();
    let target = Target::create(target)?;
    let mut plan = plan::plan(opts.seed, opts.files, opts.bulk_ms);

    for dir in plan.folders() {
        target.create_dir(&dir)?;
    }
    let sizes = write_all(&target, &plan, opts.threads)?;
    for (f, size) in plan.files.iter_mut().zip(&sizes) {
        f.entry.bytes = *size;
    }

    let manifest = manifest_of(plan, opts.seed);
    let json = serde_json::to_vec_pretty(&manifest).map_err(Error::Json)?;
    let name = RelPath::new(&[manifest::FILE_NAME])?;
    target.write_new(&name, &json)?;

    Ok(Summary {
        files: manifest.counts.files,
        bytes: manifest.counts.bytes,
        elapsed: start.elapsed(),
    })
}

/// Renders and writes every file, in parallel. Returns each file's size, in
/// plan order.
fn write_all(target: &Target, plan: &Plan, threads: usize) -> Result<Vec<u64>, SandboxError> {
    let threads = match threads {
        0 => std::thread::available_parallelism().map_or(4, |n| n.get()),
        n => n,
    };
    let next = AtomicUsize::new(0);
    let results: Vec<Result<Vec<(usize, u64)>, SandboxError>> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(f) = plan.files.get(i) else { break };
                        let bytes = render(&f.body);
                        target.write_new(&f.path, &bytes)?;
                        done.push((i, bytes.len() as u64));
                    }
                    Ok(done)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().expect("a worker panicked"))
            .collect()
    });
    let mut sizes = vec![0; plan.files.len()];
    for r in results {
        for (i, size) in r? {
            sizes[i] = size;
        }
    }
    Ok(sizes)
}

fn manifest_of(plan: Plan, seed: u64) -> Manifest {
    let duplicate_groups = plan.duplicate_groups();
    let files: Vec<_> = plan.files.into_iter().map(|f| f.entry).collect();
    let count = |role| files.iter().filter(|f| f.role == role).count();
    let counts = Counts {
        files: files.len(),
        audio: count(Role::Audio),
        traps: count(Role::Trap),
        skips: count(Role::Skip),
        bulk: files
            .iter()
            .filter(|f| f.cases.iter().any(|c| c == "bulk"))
            .count(),
        bytes: files.iter().map(|f| f.bytes).sum(),
    };
    Manifest {
        schema: manifest::SCHEMA,
        generator: format!("fixture-gen {}", env!("CARGO_PKG_VERSION")),
        seed,
        music_root: manifest::MUSIC_ROOT,
        counts,
        recordings: plan.recordings,
        duplicate_groups,
        version_links: plan.version_links,
        not_related: plan.not_related,
        files,
    }
}
