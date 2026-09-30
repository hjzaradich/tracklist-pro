//! Shared helpers: generate a tree once per test binary, decode files with
//! an independent decoder (Symphonia), walk trees.

#![allow(dead_code)]

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use fixture_gen::{generate, Options};
use serde_json::Value;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Bulk files added on top of the core cases in the shared tree: enough to
/// exercise every bulk format, small enough for CI.
pub const SHARED_BULK: usize = 60;

pub struct Fixture {
    /// The target folder, as a `\\?\` path on Windows.
    pub root: PathBuf,
    pub manifest: Value,
}

impl Fixture {
    /// The on-disk path of a manifest `path` (`/`-separated). Joined part by
    /// part onto the verbatim root, so awkward names resolve exactly.
    pub fn path(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/') {
            p.push(part);
        }
        p
    }

    pub fn files(&self) -> &Vec<Value> {
        self.manifest["files"].as_array().expect("files array")
    }

    pub fn file(&self, rel: &str) -> &Value {
        self.files()
            .iter()
            .find(|f| f["path"] == rel)
            .unwrap_or_else(|| panic!("{rel} not in manifest"))
    }

    pub fn with_case(&self, case: &str) -> Vec<&Value> {
        self.files()
            .iter()
            .filter(|f| f["cases"].as_array().unwrap().iter().any(|c| c == case))
            .collect()
    }

    pub fn files_of(&self, recording: &str) -> Vec<&Value> {
        self.files()
            .iter()
            .filter(|f| f["recording"] == recording)
            .collect()
    }
}

/// A fresh, empty folder under Cargo's per-test-binary temp folder.
pub fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        // Through the verbatim path, so names with trailing dots are removable.
        fs::remove_dir_all(fs::canonicalize(&dir).unwrap()).expect("clear old fixture folder");
    }
    dir
}

pub fn generate_into(name: &str, seed: u64, files: usize) -> Fixture {
    let dir = fresh_dir(name);
    let opts = Options {
        seed,
        files,
        ..Options::default()
    };
    generate(&dir, &opts).expect("generation succeeds");
    let root = fs::canonicalize(&dir).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("fixture-manifest.json")).unwrap()).unwrap();
    Fixture { root, manifest }
}

/// The number of core (ground-truth) files, from a plan with no bulk.
pub fn core_len() -> usize {
    fixture_gen::plan::plan(1, 0, 500).files.len()
}

/// One tree per test binary, shared by its tests.
pub fn shared() -> &'static Fixture {
    static FX: OnceLock<Fixture> = OnceLock::new();
    FX.get_or_init(|| generate_into("shared", 1, core_len() + SHARED_BULK))
}

#[derive(Debug)]
pub struct Decoded {
    /// Symphonia's short name for the container it detected from the bytes.
    pub container: &'static str,
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<i16>,
}

/// Decodes a whole file with no extension hint, so the format is detected
/// from the bytes. Any decode error fails: valid fixtures decode cleanly.
pub fn decode(path: &Path) -> Result<Decoded, String> {
    let file = File::open(path).map_err(|e| format!("open: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    let container = format.format_info().short_name;
    let track = format
        .first_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let params = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(a)) => a.clone(),
        _ => return Err("no audio codec parameters".into()),
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;
    let rate = params.sample_rate.ok_or("no sample rate")?;
    let channels = params.channels.as_ref().map_or(0, |c| c.count());

    let mut samples = Vec::new();
    let mut buf: Vec<i16> = Vec::new();
    while let Some(packet) = format.next_packet().map_err(|e| format!("read: {e}"))? {
        if packet.track_id != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("decode: {e}"))?;
        decoded.copy_to_vec_interleaved(&mut buf);
        samples.extend_from_slice(&buf);
    }
    Ok(Decoded {
        container,
        rate,
        channels,
        samples,
    })
}

/// Every file under `root`: (path parts relative to root, bytes), sorted.
pub fn walk(root: &Path) -> Vec<(Vec<OsString>, Vec<u8>)> {
    fn go(dir: &Path, prefix: &[OsString], out: &mut Vec<(Vec<OsString>, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let mut parts = prefix.to_vec();
            parts.push(e.file_name());
            if e.file_type().unwrap().is_dir() {
                go(&e.path(), &parts, out);
            } else {
                out.push((parts, fs::read(e.path()).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    go(root, &[], &mut out);
    out.sort();
    out
}

/// Normalized cross-correlation of `a` and `b[offset..]`, over `a`'s length.
pub fn correlation(a: &[i16], b: &[i16], offset: usize) -> f64 {
    let n = a.len().min(b.len().saturating_sub(offset));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let x = f64::from(a[i]);
        let y = f64::from(b[i + offset]);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa == 0.0 || bb == 0.0 {
        return 0.0;
    }
    ab / (aa.sqrt() * bb.sqrt())
}
