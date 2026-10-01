//! The one file a send writes, and writing it so a failed send leaves the
//! last one in place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::rekordbox_write::{self, Outgoing};
use crate::write_guard::{GuardError, WriteGuard};

/// The send file's name. It sits directly in the app data folder.
pub const SEND_FILE_NAME: &str = "tracklist-pro.xml";

/// Where every send is written: one fixed file in the app data folder,
/// replaced each time. The user points rekordbox at it once.
pub fn send_path(guard: &WriteGuard) -> PathBuf {
    guard.app_data_dir().join(SEND_FILE_NAME)
}

/// Why a send's file step stopped.
#[derive(Debug)]
pub enum WriteError<E> {
    /// The file couldn't be written. The file from the last send, if any,
    /// is as it was, and nothing was recorded.
    Write(GuardError),
    /// The file was written but `record` failed.
    Record {
        error: E,
        /// Whether the file is back as it was before this send. False
        /// when there was no file before (the new one stays; the next
        /// send replaces it), or when putting the old one back failed.
        put_back: bool,
    },
}

/// Writes `send` to `dest` through the write guard, then runs `record`
/// (the send's database record). First it clears the new-file leftovers a
/// crashed send could have left beside `dest`.
///
/// If the write fails, `dest` still holds the last send and `record`
/// never runs. If `record` fails, the last send's bytes are put back, so
/// the file and the record keep agreeing; when there was no file before,
/// the new file stays where it is.
pub fn write_and_record<T, E>(
    guard: &WriteGuard,
    dest: &Path,
    send: &Outgoing,
    record: impl FnOnce() -> Result<T, E>,
) -> Result<T, WriteError<E>> {
    // A leftover that can't be cleared doesn't stop the send: the write
    // takes the next free name, or fails and says so.
    if let Err(e) = guard.remove_leftover_parts(dest) {
        eprintln!("send: couldn't clear an earlier send's leftovers: {e}");
    }
    let before = match fs::read(dest) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(WriteError::Write(GuardError::Io {
                path: dest.to_owned(),
                source,
            }))
        }
    };
    rekordbox_write::write_file(guard, dest, send).map_err(WriteError::Write)?;
    record().map_err(|error| {
        let put_back = match &before {
            Some(bytes) => match guard.write_then_rename(dest, bytes) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("send: couldn't put the last send's file back: {e}");
                    false
                }
            },
            None => false,
        };
        WriteError::Record { error, put_back }
    })
}
