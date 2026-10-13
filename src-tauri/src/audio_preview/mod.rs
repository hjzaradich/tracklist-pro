//! Preview over a custom Tauri protocol (1cA-7, 1cA-8; ROADMAP 1.11
//! "Audio").
//!
//! The player is an audio element in the webview. Its `src` is
//! `http://audio-preview.localhost/<file id>` (the spelling WebView2 gives
//! the `audio-preview` scheme; the page builds it with `convertFileSrc`).
//! The URL names a file by its id and nothing else: the path has to be `/`
//! and digits, and the id is turned into a path here, from the database,
//! the way every job turns a file row into a path. A file the app hasn't
//! scanned, in a music folder it doesn't know, is refused. There is no
//! HTTP server and no socket: the webview asks Tauri, Tauri asks this
//! module, and the answer goes back in memory.
//!
//! - **Formats the webview plays itself** (MP3, AAC in M4A or ADTS, FLAC,
//!   WAV, Ogg Vorbis and Opus) are served as they are on disk, with Range
//!   support. Tauri answers a custom protocol with a whole body in memory,
//!   so a response never holds more than [`MAX_CHUNK`] bytes: a request for
//!   more (`bytes=0-`, or none) gets the first chunk as a 206, and the media
//!   element asks for the next. A file is never read whole.
//! - **AIFF and ALAC** are decoded in Rust ([`wav`]): the answer is a WAV
//!   whose header is computed from the container, and each range is decoded
//!   from where it starts, so seeking works and memory stays bounded.
//! - **Anything else**, and anything that can't be decoded, is refused as
//!   [`PreviewRefusal::NotDecodable`].
//! - **The handler never works on the webview thread.** It hands each
//!   request to one of a few worker threads ([`workers`]) and answers from
//!   there. Preview is interactive, so it doesn't go through the job queue.
//!
//! Nothing here writes: files are opened read-only, nothing is cached or
//! kept in a temp file, and the Rust side holds no playback state (the
//! position and play/pause live in the frontend's player store).
//!
//! A refusal is a typed [`PreviewRefusal`] code, never English text. A media
//! element can't read the status of a failed request, so the player asks
//! [`preview_check`], which runs the same [`open::open`], for the code.

mod open;
mod serve;
mod wav;
mod workers;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Builder, Manager, Runtime, State};

use crate::db::ReadPool;

/// The protocol's name. WebView2 on Windows spells it
/// `http://audio-preview.localhost`, which is the one media source the
/// page's content security policy allows (`tauri.conf.json`,
/// `tests/csp.rs`).
pub const SCHEME: &str = "audio-preview";

/// The most bytes one response holds (1 MiB). Tauri's own asset protocol
/// cuts its answers at about the same size.
pub const MAX_CHUNK: u64 = 1024 * 1024;

/// Why a file can't be previewed. The player shows a line for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PreviewRefusal {
    /// No such file, or not in a music folder the app knows.
    UnknownFile,
    /// The last scan didn't find it, or it isn't there now.
    Missing,
    /// Its drive isn't connected.
    DriveNotConnected,
    /// A OneDrive file that isn't on this PC. Never opened: reading it
    /// would download it.
    OnlineOnly,
    /// The OS wouldn't read it (locked, no access, an I/O error).
    Unreadable,
    /// Not a format the player can play, or damaged beyond decoding.
    NotDecodable,
}

impl PreviewRefusal {
    /// The code as it is in the bindings and the response header.
    pub fn code(self) -> &'static str {
        match self {
            PreviewRefusal::UnknownFile => "unknown_file",
            PreviewRefusal::Missing => "missing",
            PreviewRefusal::DriveNotConnected => "drive_not_connected",
            PreviewRefusal::OnlineOnly => "online_only",
            PreviewRefusal::Unreadable => "unreadable",
            PreviewRefusal::NotDecodable => "not_decodable",
        }
    }
}

/// Whether file `file_id` can be played, and if not why: `None` means it
/// can. The same check the protocol makes before it answers, so the player
/// can show the reason when its audio element reports a failure.
#[tauri::command(async)]
#[specta::specta]
pub fn preview_check(reads: State<'_, ReadPool>, file_id: i64) -> Option<PreviewRefusal> {
    check(|| {
        let volumes = crate::scan::system_volumes();
        open::open(&reads, &volumes, file_id).map(drop)
    })
}

/// Runs a check with the guard the protocol handler has: a panic (the
/// probe is third-party code) is an answer, `unreadable`, not a reply that
/// never comes and a player stuck on "loading".
fn check(work: impl FnOnce() -> Result<(), PreviewRefusal>) -> Option<PreviewRefusal> {
    serve::contained_as(PreviewRefusal::Unreadable, work).err()
}

/// Registers the `audio-preview` protocol on the builder.
pub fn register<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    let workers = workers::Workers::new();
    builder.register_asynchronous_uri_scheme_protocol(SCHEME, move |ctx, request, responder| {
        let app = ctx.app_handle().clone();
        // The webview thread's whole job is to hand the request over.
        workers.run(move || {
            // Everything that builds the reply is inside the guard, so the
            // request is answered whatever happens.
            let reply = serve::guarded(|| match app.try_state::<ReadPool>() {
                Some(reads) => {
                    let volumes = crate::scan::system_volumes();
                    serve::answer(&reads, &volumes, &request)
                }
                None => serve::refused(PreviewRefusal::Unreadable),
            });
            responder.respond(reply);
        });
    })
}
