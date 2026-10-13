//! From a file id to an open file the preview can answer from, or the typed
//! reason it can't (1cA-7). The one place that decides, so the protocol
//! handler and the `preview_check` command can't disagree.

use std::fs::File;
use std::io::{self, Seek, SeekFrom};
use std::path::Path;

use rusqlite::OptionalExtension;

use super::wav::Decoded;
use super::PreviewRefusal;
use crate::db::ReadPool;
use crate::paths::{PathError, RelPath, Volumes};
use crate::read::{codec_of, Codec};
use crate::scan::folders;
use crate::scan::online_only::is_online_now;
use crate::sniff::{sniff, SniffedFormat};

/// A file ready to be answered from.
pub(super) enum Opened {
    /// A format the webview plays itself: bytes are served as they are.
    Native {
        file: File,
        len: u64,
        mime: &'static str,
    },
    /// AIFF or ALAC: served as a WAV made on the fly.
    Decoded(Box<Decoded>),
}

/// What the database says about a file id.
struct Row {
    folder: i64,
    rel_path: String,
    present: bool,
    online_only: bool,
}

/// Opens file `file_id` read-only for preview.
///
/// Refused unless it is a scanned file (a `file` row) in a music folder the
/// app knows. The path is built from the row exactly as the jobs build it
/// (the volume's mount point now, plus the stored path), never from
/// anything the page sent. In order, so a refusal opens nothing it didn't
/// need to: an unknown row; a file the last scan didn't find; a drive that
/// isn't connected; an online-only OneDrive file (the row says so, or its
/// attributes do right now, whatever the user's opt-in: preview never
/// downloads); then the open itself.
///
/// The file is played as it is on disk now. Nothing derived from it is
/// stored, so the size and date its row holds aren't checked.
pub(super) fn open(
    reads: &ReadPool,
    volumes: &impl Volumes,
    file_id: i64,
) -> Result<Opened, PreviewRefusal> {
    let (row, folders) = reads
        .read(|conn| {
            let row = conn
                .query_row(
                    "SELECT music_folder_id, rel_path, present, online_only
                     FROM file WHERE id = ?1",
                    [file_id],
                    |r| {
                        Ok(Row {
                            folder: r.get(0)?,
                            rel_path: r.get(1)?,
                            present: r.get(2)?,
                            online_only: r.get(3)?,
                        })
                    },
                )
                .optional()?;
            // Only the one folder is needed, but they are read through the
            // one parsing path.
            let folders = match &row {
                Some(_) => folders::stored(conn)?,
                None => Vec::new(),
            };
            Ok((row, folders))
        })
        .map_err(|_| PreviewRefusal::Unreadable)?;
    let row = row.ok_or(PreviewRefusal::UnknownFile)?;
    let folder = folders
        .iter()
        .find(|f| f.id.0 == row.folder)
        .ok_or(PreviewRefusal::UnknownFile)?;
    let rel = RelPath::parse(&row.rel_path).map_err(|_| PreviewRefusal::UnknownFile)?;
    if !row.present {
        return Err(PreviewRefusal::Missing);
    }
    let path = match folder.stored_path().join(&rel).resolve(volumes) {
        Ok(path) => path,
        Err(PathError::Offline(_)) => return Err(PreviewRefusal::DriveNotConnected),
        Err(_) => return Err(PreviewRefusal::UnknownFile),
    };
    if row.online_only {
        return Err(PreviewRefusal::OnlineOnly);
    }
    match is_online_now(&path) {
        Ok(false) => {}
        Ok(true) => return Err(PreviewRefusal::OnlineOnly),
        Err(e) => return Err(io_refusal(&e)),
    }
    // A folder where the file was is a file that is gone, not one the OS
    // won't read.
    if !std::fs::metadata(&path)
        .map_err(|e| io_refusal(&e))?
        .is_file()
    {
        return Err(PreviewRefusal::Missing);
    }
    let mut file = File::open(&path).map_err(|e| io_refusal(&e))?;
    let meta = file.metadata().map_err(|e| io_refusal(&e))?;
    let kind = classify(&mut file, &path, meta.len())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| io_refusal(&e))?;
    Ok(match kind {
        Kind::Native(mime) => Opened::Native {
            file,
            len: meta.len(),
            mime,
        },
        Kind::Decode => Opened::Decoded(Box::new(Decoded::from_source(Box::new(file))?)),
    })
}

/// A file that isn't there is missing; anything else the OS says is
/// unreadable.
fn io_refusal(e: &io::Error) -> PreviewRefusal {
    match e.kind() {
        io::ErrorKind::NotFound => PreviewRefusal::Missing,
        _ => PreviewRefusal::Unreadable,
    }
}

enum Kind {
    /// Served as it is, with this media type.
    Native(&'static str),
    /// Decoded to WAV.
    Decode,
}

/// How the file is played, from its bytes (never its extension): the
/// webview's own formats are served as they are, AIFF and ALAC are decoded,
/// and anything else can't be played.
fn classify(file: &mut File, path: &Path, len: u64) -> Result<Kind, PreviewRefusal> {
    if len == 0 {
        return Err(PreviewRefusal::NotDecodable);
    }
    let extension = path.extension().and_then(|e| e.to_str());
    let sniffed = sniff(file, extension).map_err(|e| io_refusal(&e))?;
    let codec = codec_of(file, sniffed.format).map_err(|e| io_refusal(&e))?;
    use SniffedFormat as F;
    Ok(match (sniffed.format, codec) {
        (F::Mp3, _) => Kind::Native("audio/mpeg"),
        (F::Flac, _) => Kind::Native("audio/flac"),
        (F::Adts, _) => Kind::Native("audio/aac"),
        (F::OggVorbis | F::OggOpus, _) => Kind::Native("audio/ogg"),
        (F::Ogg, Some(Codec::Vorbis | Codec::Opus)) => Kind::Native("audio/ogg"),
        (F::Wav, Some(Codec::Pcm | Codec::PcmFloat | Codec::Alaw | Codec::Mulaw)) => {
            Kind::Native("audio/wav")
        }
        (F::Mp4, Some(Codec::Aac)) => Kind::Native("audio/mp4"),
        (F::Mp4, Some(Codec::Alac)) => Kind::Decode,
        (F::Aiff | F::Aifc, _) => Kind::Decode,
        _ => return Err(PreviewRefusal::NotDecodable),
    })
}
