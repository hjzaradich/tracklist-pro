//! Stage 3 of the scan, part one: each file's `blake3` and `audio_hash`
//! (1aB-5, 1aB-6; ROADMAP 1.1, §2, §5.1).
//!
//! - `blake3` is the BLAKE3 hash of every byte of the file (32 bytes).
//! - `audio_hash` covers the audio frames only, so a file whose tags were
//!   rewritten keeps it. What it covers, format by format, is defined in
//!   [`audio`]; it's stored with a definition and format prefix.
//!
//! Both come from one pass over the file: [`hash_reader`] first works out
//! where the audio is from the headers (a few small reads), then streams
//! the file once through a fixed [`BUFFER`], feeding every byte to the
//! whole-file hasher and the audio bytes to the audio hasher. Memory stays
//! bounded however big the file is, and cancelling is checked before every
//! buffer, so a 2 GB file stops within one buffer.
//!
//! Files are only read. The job ([`job`]) writes to the database alone.

pub mod audio;
mod job;
pub mod partial;
mod state;

pub use audio::{AudioFormat, AudioHash, Skip, AUDIO_HASH_LEN, DEFINITION};
pub use job::{hash_job, Hasher, Summary, BATCH_MAX, BATCH_WINDOW};

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use tauri::State;

use crate::ipc::IpcError;
use crate::jobs::{JobId, JobQueue};
use crate::scan::MusicFolderId;

/// How much of a file is read at a time.
pub const BUFFER: usize = 1 << 20;

/// What hashing one file gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHashes {
    /// BLAKE3 of the whole file.
    pub blake3: [u8; 32],
    /// The audio-only hash, or why there's none.
    pub audio: Result<AudioHash, Skip>,
    /// How many bytes were read.
    pub len: u64,
}

/// Hashes the file in `r` from its start, reading `buf.len()` bytes at a
/// time. Before each read, `stop` is called with how many bytes have been
/// read so far; if it says true, hashing stops and `Ok(None)` is returned.
///
/// A file that ends early, or whose audio can't be found, still gets its
/// `blake3`; only its audio is a [`Skip`]. I/O errors are returned.
pub fn hash_reader<R: Read + Seek>(
    r: &mut R,
    buf: &mut [u8],
    stop: &mut dyn FnMut(u64) -> bool,
) -> io::Result<Option<FileHashes>> {
    assert!(!buf.is_empty(), "hash_reader needs a buffer");
    let planned_len = r.seek(SeekFrom::End(0))?;
    let plan = audio::plan(r, planned_len)?;
    let mut sink = audio::Sink::new(plan);
    let mut whole = blake3::Hasher::new();
    r.seek(SeekFrom::Start(0))?;
    let mut offset = 0u64;
    loop {
        if stop(offset) {
            return Ok(None);
        }
        let n = match r.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        whole.update(&buf[..n]);
        sink.feed(offset, &buf[..n]);
        offset += n as u64;
    }
    let audio = if offset == planned_len {
        sink.finish(offset)
    } else {
        // The file changed size while it was read; the plan is stale.
        Err(Skip::Truncated)
    };
    Ok(Some(FileHashes {
        blake3: *whole.finalize().as_bytes(),
        audio,
        len: offset,
    }))
}

/// Opens `path` read-only and hashes it (see [`hash_reader`]). `path` must
/// be absolute; on Windows it's opened in its `\\?\` form (ROADMAP §5.6).
pub fn hash_path(
    path: &Path,
    buf: &mut [u8],
    stop: &mut dyn FnMut(u64) -> bool,
) -> io::Result<Option<FileHashes>> {
    // `File::open` asks for read access only.
    let mut file = File::open(open_form(path)?)?;
    hash_reader(&mut file, buf, stop)
}

#[cfg(windows)]
pub(crate) fn open_form(path: &Path) -> io::Result<std::path::PathBuf> {
    crate::paths::verbatim_absolute(path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[cfg(not(windows))]
pub(crate) fn open_form(path: &Path) -> io::Result<std::path::PathBuf> {
    Ok(path.to_path_buf())
}

/// Hashes the files in the music folders `ids`, or in all of them, that
/// have no hashes yet or changed since they were hashed. Returns the job's
/// id; it runs in the background at low priority, and its progress shows
/// in Activity.
#[tauri::command(async)]
#[specta::specta]
pub fn hash_music_folders(
    jobs: State<'_, JobQueue>,
    ids: Option<Vec<MusicFolderId>>,
) -> Result<JobId, IpcError> {
    Ok(jobs.enqueue(hash_job(ids))?)
}

/// The hash job's handler for the app: asks Windows which volumes are
/// mounted at the start of each job.
pub fn hasher() -> impl crate::jobs::JobHandler {
    Hasher::new(crate::scan::system_volumes)
}

#[cfg(test)]
mod tests;
