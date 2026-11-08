//! Cover art for a look-up's releases (ROADMAP 2.2; Run 6A, S2b).
//!
//! The screen asks for one release's front cover ([`identify_cover`]) and
//! shows it; the app keeps it nowhere. Images wait in a small cache in memory
//! ([`CoverCache`]: at most [`MAX_IMAGES`] images and [`MAX_BYTES`] bytes,
//! the oldest out first), so going back to a track doesn't ask the archive
//! again, and the cache is gone when the app closes. No image is ever
//! written to disk, not here and not by anything this module calls.
//!
//! The request goes through the gate (Cover Art must be switched on) and the
//! release id is checked to be a UUID before it is put in an address.
//! Artwork is the owner's of the release; the app only shows it (ROADMAP
//! 5.7). Discogs images are never fetched.
//!
//! [`identify_cover_releases`] and [`identify_file_cover_releases`] say
//! which release ids a kept look-up answer has, so a screen knows what it
//! can ask for.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock, PoisonError};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;
use tauri::State;

use super::cache;
use super::lookup::Outcome;
use crate::db::{DbError, ReadPool};
use crate::ipc::IpcError;
use crate::library::LibraryTrackId;
use crate::net::services::coverart::{self, Artwork};
use crate::net::services::ClientError;
use crate::net::{is_opted_in, Net, Service};

/// The most images kept.
pub const MAX_IMAGES: usize = 50;
/// The most bytes kept, all images together.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

/// Whether `id` is a UUID in the 8-4-4-4-12 form MusicBrainz uses.
pub fn is_release_id(id: &str) -> bool {
    crate::net::services::is_mbid(id)
}

/// Images by release id, oldest first. In memory only.
#[derive(Default)]
pub struct CoverCache {
    entries: Mutex<VecDeque<(String, Artwork)>>,
}

impl CoverCache {
    pub fn new() -> CoverCache {
        CoverCache::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, Artwork)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn get(&self, release: &str) -> Option<Artwork> {
        let entries = self.lock();
        entries
            .iter()
            .find(|(id, _)| id == release)
            .map(|(_, a)| a.clone())
    }

    /// Keeps `art`, pushing the oldest images out until the limits hold. An
    /// image over [`MAX_BYTES`] on its own is not kept.
    pub fn put(&self, release: &str, art: Artwork) {
        if art.bytes.len() > MAX_BYTES {
            return;
        }
        let mut entries = self.lock();
        entries.retain(|(id, _)| id != release);
        entries.push_back((release.to_owned(), art));
        let mut bytes: usize = entries.iter().map(|(_, a)| a.bytes.len()).sum();
        while entries.len() > MAX_IMAGES || bytes > MAX_BYTES {
            let Some((_, gone)) = entries.pop_front() else {
                break;
            };
            bytes -= gone.bytes.len();
        }
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn bytes(&self) -> usize {
        self.lock().iter().map(|(_, a)| a.bytes.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

/// The app's one cache.
fn shared() -> &'static CoverCache {
    static CACHE: OnceLock<CoverCache> = OnceLock::new();
    CACHE.get_or_init(CoverCache::new)
}

/// What asking for a cover came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoverAnswer {
    /// The picture: `data` is base64 of the image, `mime` its type
    /// (`image/jpeg`), for a `data:` address.
    Image { mime: String, data: String },
    /// The archive has no cover for the release.
    NoCover,
    /// Nothing was asked: `reason` is `bad_id` or `cover_art_off`.
    Refused { reason: String },
    /// The request failed: `reason` is a short code (`offline`, `busy`,
    /// `failed`).
    Failed { reason: String },
}

fn refused(reason: &str) -> CoverAnswer {
    CoverAnswer::Refused {
        reason: reason.to_owned(),
    }
}

fn image(art: &Artwork) -> CoverAnswer {
    CoverAnswer::Image {
        mime: art.mime.clone(),
        data: STANDARD.encode(&art.bytes),
    }
}

/// [`identify_cover`] with the fetch and the cache given. Checks the id and
/// the switch first, even for an image the cache has.
pub fn cover_with(
    reads: &ReadPool,
    cache: &CoverCache,
    release: &str,
    fetch: impl Fn(&str) -> Result<Option<Artwork>, ClientError>,
) -> Result<CoverAnswer, DbError> {
    let release = release.trim().to_ascii_lowercase();
    if !is_release_id(&release) {
        return Ok(refused("bad_id"));
    }
    if !is_opted_in(reads, Service::CoverArt)? {
        return Ok(refused("cover_art_off"));
    }
    if let Some(art) = cache.get(&release) {
        return Ok(image(&art));
    }
    Ok(match fetch(&release) {
        Ok(Some(art)) => {
            let answer = image(&art);
            cache.put(&release, art);
            answer
        }
        Ok(None) => CoverAnswer::NoCover,
        Err(e) => failure(&e),
    })
}

fn failure(error: &ClientError) -> CoverAnswer {
    use crate::net::NetError;
    let reason = match error {
        ClientError::Net(NetError::NotOptedIn(_)) => return refused("cover_art_off"),
        ClientError::Net(e) if e.is_offline() => "offline",
        ClientError::RateLimited(_) | ClientError::Status { status: 503, .. } => "busy",
        _ => "failed",
    };
    CoverAnswer::Failed {
        reason: reason.to_owned(),
    }
}

/// The front cover of a MusicBrainz release, from the Cover Art Archive,
/// through the gate. Answers what came of it; never an error for a refusal.
/// The image is kept in memory only.
#[tauri::command(async)]
#[specta::specta]
pub fn identify_cover(
    reads: State<'_, ReadPool>,
    net: State<'_, Net>,
    release_mbid: String,
) -> Result<CoverAnswer, IpcError> {
    Ok(cover_with(&reads, shared(), &release_mbid, |id| {
        coverart::front(&net, id)
    })?)
}

/// The release ids a file's kept look-up found (when it found details), in
/// the order they were found.
pub fn releases_of_file(conn: &Connection, file: i64) -> rusqlite::Result<Vec<String>> {
    Ok(cache::load(conn, file)?
        .filter(|c| c.outcome == Outcome::Found && !c.stale)
        .map(|c| c.releases)
        .unwrap_or_default()
        .into_iter()
        .filter(|id| is_release_id(id))
        .collect())
}

/// The release ids of a Library track's linked file's kept look-up.
pub fn releases_of_track(
    conn: &Connection,
    track: LibraryTrackId,
) -> rusqlite::Result<Vec<String>> {
    let file: Option<Option<i64>> = conn
        .query_row(
            "SELECT linked_file_id FROM library_track WHERE id = ?1",
            [track.0],
            |r| r.get(0),
        )
        .optional()?;
    match file.flatten() {
        Some(file) => releases_of_file(conn, file),
        None => Ok(Vec::new()),
    }
}

/// The releases a track's kept look-up has, so the screen can ask for a
/// cover. Makes no request.
#[tauri::command]
#[specta::specta]
pub async fn identify_cover_releases(
    reads: State<'_, ReadPool>,
    library_track_id: LibraryTrackId,
) -> Result<Vec<String>, IpcError> {
    Ok(reads.read(|conn| releases_of_track(conn, library_track_id))?)
}

/// [`identify_cover_releases`] for an Inbox file.
#[tauri::command]
#[specta::specta]
pub async fn identify_file_cover_releases(
    reads: State<'_, ReadPool>,
    file_id: i64,
) -> Result<Vec<String>, IpcError> {
    Ok(reads.read(|conn| releases_of_file(conn, file_id))?)
}
