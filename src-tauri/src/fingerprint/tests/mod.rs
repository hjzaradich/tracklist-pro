//! Behavior tests for fingerprints (1aB-7). The audio is generated in code
//! (`audio`), never committed.

mod audio;
mod decoding;
mod format;
#[cfg(windows)]
mod job;
mod matching;
mod speed;
#[cfg(windows)]
mod support;

use std::io::Cursor;

use super::decode::{self, Stopped};
use super::Fingerprint;

/// Fingerprints `bytes` as a file holding them would be.
fn decode_bytes(bytes: &[u8]) -> Result<Fingerprint, Stopped> {
    decode::fingerprint(Box::new(Cursor::new(bytes.to_vec())), &mut |_| true)
}

#[test]
fn the_frontend_can_start_fingerprinting_and_put_the_visible_tracks_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    crate::ipc::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();
    for expected in [
        r#"fingerprintFiles: () => typedError<JobId, IpcError>(__TAURI_INVOKE("fingerprint_files"))"#,
        r#"fingerprintFirst: (fileIds: number[]) => typedError<number | null, IpcError>(__TAURI_INVOKE("fingerprint_first", { fileIds }))"#,
    ] {
        assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
    }
}
