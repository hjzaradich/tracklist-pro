//! The listening step (ROADMAP 3.8): the app's own audio model, CED-tiny
//! (Apache-2.0), run on the CPU in pure Rust with `tract`.
//!
//! - **What it makes.** For one track: a fingerprint of 192 numbers (what
//!   the track sounds like, to compare tracks by) and 527 label scores (what
//!   AudioSet's labels the model hears, before any sigmoid), plus where in
//!   the track it listened.
//! - **How.** Three 10 s windows, centred at 25, 50 and 75 % of the track
//!   ([`window`]); each is read by seeking, so a long track costs the same
//!   as a short one ([`decode`]); averaged to mono; resampled to 16 kHz
//!   ([`resample`]); turned into 64 x 1001 mel features exactly as CED's own
//!   code does ([`mel`]); run through the model. The fingerprint is the mean
//!   of the three embeddings and the scores the mean of the three windows'.
//! - **Loaded once.** [`AudioModel`] is built from the bundled model file
//!   (read into memory, its sha256 checked against [`MODEL_SHA256`] first,
//!   never extracted to disk) and shared by every worker thread.
//! - **Reads only.** Nothing here writes a file or opens a connection; a
//!   track is opened read-only.
//!
//! The embed job ([`job`]) listens to each Library track with it and keeps
//! the result ([`ledger`], [`subset`]); [`control`] holds the switch (off by
//! default, nothing loaded while off) and the status. No screen lives here.

pub mod control;
pub mod decode;
pub mod job;
pub mod ledger;
pub mod mel;
pub mod resample;
pub mod subset;
pub mod window;

pub(crate) use control::{due, request};
pub use control::{embedder, switch, AudioModelStatus};
pub use job::{embed_job, Embedder, PRIORITY};

#[cfg(test)]
mod tests;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, Runtime};
use tract_onnx::prelude::*;

/// Length of a track's fingerprint.
pub const EMBEDDING_LEN: usize = 192;
/// How many label scores a track has.
pub const LABEL_COUNT: usize = 527;
/// The highest sample rate accepted from a file. Buffers are sized from the
/// rate a header claims, so a corrupt or hostile header can't be believed.
pub const MAX_RATE: u32 = 384_000;
/// The sha256 of the one model file this code is written for.
pub const MODEL_SHA256: &str = "a455633ff46e39382f5847a4b1a825b90ff40f1cc3f50c09d59f3d4ef06f0886";
/// Where the installer puts the model, under the app's resource folder
/// (`bundle.resources` in `tauri.conf.json`).
const BUNDLED: [&str; 2] = [
    "resources/models/ced-tiny/ced-tiny.onnx",
    "models/ced-tiny/ced-tiny.onnx",
];

/// Why the model couldn't be loaded.
#[derive(Debug)]
pub enum ModelError {
    /// There is no model file at this path.
    Missing(PathBuf),
    /// The file couldn't be read.
    Unreadable(String),
    /// The file isn't the model this build was written for (its sha256 is
    /// not [`MODEL_SHA256`]): damaged, replaced or the wrong version.
    NotTheModel { found: String },
    /// The file is the right one but the engine couldn't prepare it.
    Engine(String),
    /// The installer's resource folder couldn't be found.
    NoResourceFolder(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::Missing(p) => write!(f, "the audio model file is missing: {}", p.display()),
            ModelError::Unreadable(e) => write!(f, "the audio model file can't be read: {e}"),
            ModelError::NotTheModel { found } => {
                write!(
                    f,
                    "the audio model file isn't the expected one (sha256 {found})"
                )
            }
            ModelError::Engine(e) => write!(f, "the audio model can't be prepared: {e}"),
            ModelError::NoResourceFolder(e) => {
                write!(f, "the app's resource folder wasn't found: {e}")
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// Why one track couldn't be listened to.
#[derive(Debug)]
pub enum ListenError {
    /// The file couldn't be opened or read (it may come back later).
    Unreadable(String),
    /// No audio track, or none of a usable rate or length.
    NoAudio,
    /// A format or codec this build can't decode.
    Unsupported,
    /// The file claims a sample rate above [`MAX_RATE`].
    ImpossibleRate(u32),
    /// The file is too damaged to read.
    Damaged,
    /// `keep_going` said stop.
    Cancelled,
    /// The model failed on a window (a bug, not a property of the track).
    Model(String),
}

impl fmt::Display for ListenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenError::Unreadable(e) => write!(f, "the track can't be read: {e}"),
            ListenError::NoAudio => write!(f, "the track has no audio"),
            ListenError::Unsupported => write!(f, "the track's format can't be decoded"),
            ListenError::ImpossibleRate(r) => {
                write!(
                    f,
                    "the track claims a sample rate of {r} Hz, which can't be real"
                )
            }
            ListenError::Damaged => write!(f, "the track is damaged"),
            ListenError::Cancelled => write!(f, "cancelled"),
            ListenError::Model(e) => write!(f, "the audio model failed: {e}"),
        }
    }
}

impl std::error::Error for ListenError {}

/// What the model heard in one track.
#[derive(Debug, Clone, PartialEq)]
pub struct Listening {
    /// [`EMBEDDING_LEN`] numbers: the mean of the three windows' embeddings.
    pub fingerprint: Vec<f32>,
    /// [`LABEL_COUNT`] scores (logits): the mean of the three windows'.
    pub scores: Vec<f32>,
    /// Where the three windows started, in seconds.
    pub window_starts: [f64; 3],
}

/// The model's two outputs for one window.
#[derive(Debug, Clone)]
pub(crate) struct Heard {
    pub emb: Vec<f32>,
    pub logits: Vec<f32>,
}

type Plan = Arc<TypedRunnableModel>;

/// CED-tiny, ready to run. Built once; `Send + Sync`, so any number of
/// threads can call [`listen`](Self::listen) on the same one.
pub struct AudioModel {
    plan: Plan,
    frontend: mel::Frontend,
}

impl AudioModel {
    /// Loads the model the installer bundled, from the app's resource folder.
    pub fn load_bundled<R: Runtime>(app: &AppHandle<R>) -> Result<AudioModel, ModelError> {
        let mut first = None;
        for name in BUNDLED {
            let path = app
                .path()
                .resolve(name, BaseDirectory::Resource)
                .map_err(|e| ModelError::NoResourceFolder(e.to_string()))?;
            if path.is_file() {
                return AudioModel::load(&path);
            }
            first.get_or_insert(path);
        }
        Err(ModelError::Missing(first.unwrap_or_default()))
    }

    /// Loads the model file at `path`. The file is checked against
    /// [`MODEL_SHA256`] before the engine sees it; a missing, unreadable or
    /// different file is an error, never a panic.
    pub fn load(path: &Path) -> Result<AudioModel, ModelError> {
        let bytes = std::fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ModelError::Missing(path.to_path_buf()),
            _ => ModelError::Unreadable(e.to_string()),
        })?;
        let found = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if found != MODEL_SHA256 {
            return Err(ModelError::NotTheModel { found });
        }
        let engine = |e: TractError| ModelError::Engine(e.to_string());
        let plan = tract_onnx::onnx()
            .model_for_read(&mut &bytes[..])
            .and_then(|m| m.with_input_fact(0, f32::fact([1, mel::N_MELS, mel::N_FRAMES]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(engine)?;
        Ok(AudioModel {
            plan,
            frontend: mel::Frontend::new(),
        })
    }

    /// Listens to the track at `path`: three windows, decoded by seeking.
    /// `keep_going` is asked often; when it says `false` the call returns
    /// [`ListenError::Cancelled`].
    pub fn listen(
        &self,
        path: &Path,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Listening, ListenError> {
        let read = decode::read_stretches(path, keep_going)?;
        self.listen_stretches(&read.stretches, read.rate, read.duration, keep_going)
    }

    /// Listens to a track already decoded: mono samples at `rate`.
    /// `keep_going` is asked before each window.
    pub fn listen_samples(
        &self,
        mono: &[f32],
        rate: u32,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Listening, ListenError> {
        let (heard, window_starts) = self.hear_samples(mono, rate, keep_going)?;
        Ok(average(&heard, window_starts))
    }

    fn listen_stretches(
        &self,
        stretches: &[Vec<f32>],
        rate: u32,
        duration: f64,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Listening, ListenError> {
        let heard = self.hear_stretches(stretches, rate, keep_going)?;
        Ok(average(&heard, window::window_starts(duration)))
    }

    pub(crate) fn hear_samples(
        &self,
        mono: &[f32],
        rate: u32,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<(Vec<Heard>, [f64; 3]), ListenError> {
        if rate == 0 || mono.is_empty() {
            return Err(ListenError::NoAudio);
        }
        if rate > MAX_RATE {
            return Err(ListenError::ImpossibleRate(rate));
        }
        let starts = window::window_starts(mono.len() as f64 / f64::from(rate));
        let stretches: Vec<Vec<f32>> = starts
            .iter()
            .map(|&s| window::stretch_of(mono, rate, s))
            .collect();
        Ok((self.hear_stretches(&stretches, rate, keep_going)?, starts))
    }

    fn hear_stretches(
        &self,
        stretches: &[Vec<f32>],
        rate: u32,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Vec<Heard>, ListenError> {
        stretches
            .iter()
            .map(|stretch| {
                if !keep_going() {
                    return Err(ListenError::Cancelled);
                }
                let wave = resample::to_window(stretch, rate).ok_or(ListenError::Unsupported)?;
                self.hear_window(&wave)
            })
            .collect()
    }

    /// One window: exactly 160 000 samples at 16 kHz.
    pub(crate) fn hear_window(&self, wave: &[f32]) -> Result<Heard, ListenError> {
        self.run(self.frontend.mel(wave))
            .map_err(ListenError::Model)
    }

    /// The model on a mel (64 x 1001, row-major).
    pub(crate) fn run(&self, mel: Vec<f32>) -> Result<Heard, String> {
        let outputs = self.outputs(mel).map_err(|e| e.to_string())?;
        let (mut emb, mut logits) = (None, None);
        // The two outputs are told apart by their length.
        for values in outputs {
            match values.len() {
                EMBEDDING_LEN => emb = Some(values),
                LABEL_COUNT => logits = Some(values),
                n => return Err(format!("the model gave an output of {n} values")),
            }
        }
        match (emb, logits) {
            (Some(emb), Some(logits)) => Ok(Heard { emb, logits }),
            _ => Err("the model didn't give both its outputs".to_string()),
        }
    }

    fn outputs(&self, mel: Vec<f32>) -> TractResult<Vec<Vec<f32>>> {
        let input = Tensor::from_shape(&[1, mel::N_MELS, mel::N_FRAMES], &mel)?;
        let outputs = self.plan.run(tvec!(input.into()))?;
        outputs
            .iter()
            .map(|o| Ok(o.try_as_plain_ram()?.as_slice::<f32>()?.to_vec()))
            .collect()
    }
}

/// The fingerprint and scores: the mean over the windows.
fn average(heard: &[Heard], window_starts: [f64; 3]) -> Listening {
    let mean = |pick: fn(&Heard) -> &Vec<f32>, len: usize| {
        let mut sum = vec![0f32; len];
        for h in heard {
            for (s, v) in sum.iter_mut().zip(pick(h)) {
                *s += v;
            }
        }
        sum.iter().map(|s| s / heard.len() as f32).collect()
    };
    Listening {
        fingerprint: mean(|h| &h.emb, EMBEDDING_LEN),
        scores: mean(|h| &h.logits, LABEL_COUNT),
        window_starts,
    }
}
