//! The commands behind Settings > Online services (ROADMAP 2.2): which
//! lookup services are on, and the user's own Discogs token.
//!
//! Every service is off until the user switches it on. The token goes into
//! the operating system's password store ([`crate::credentials`]) and
//! nowhere else. Nothing here returns it, logs it or puts it in an error:
//! the replies are plain outcomes the screen words itself.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::services::{acoustid, discogs, ClientError};
use super::{is_opted_in, set_opt_in, Net, NetError, Service};
use crate::credentials::{self, Credentials};
use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::{ErrorKind, IpcError};

/// A lookup service the user can switch on or off. Update checks and model
/// downloads have their own switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OnlineService {
    MusicBrainz,
    AcoustId,
    Discogs,
    CoverArt,
}

impl OnlineService {
    fn service(self) -> Service {
        match self {
            OnlineService::MusicBrainz => Service::MusicBrainz,
            OnlineService::AcoustId => Service::AcoustId,
            OnlineService::Discogs => Service::Discogs,
            OnlineService::CoverArt => Service::CoverArt,
        }
    }
}

/// AcoustID's switch and whether the service can work at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AcoustIdStatus {
    pub on: bool,
    /// The project's application key isn't filled in yet: the service is
    /// unavailable whatever the switch says, and nothing is sent.
    pub key_missing: bool,
}

/// Discogs' switch and its token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscogsStatus {
    pub on: bool,
    /// A token is saved in the password store.
    pub token_saved: bool,
    /// This computer has a password store to keep a token in.
    pub store_available: bool,
}

/// Where the lookup services stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OnlineServicesStatus {
    pub music_brainz: bool,
    pub acoust_id: AcoustIdStatus,
    pub discogs: DiscogsStatus,
    pub cover_art: bool,
}

/// What saving a Discogs token came to. Nothing was saved unless it says
/// `saved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SaveTokenOutcome {
    /// Discogs accepted the token and it is in the password store.
    Saved,
    /// The Discogs switch is off, so nothing was sent or saved.
    SwitchOff,
    /// The token was empty or couldn't be one.
    Empty,
    /// This computer has no password store to keep it in.
    StoreUnavailable,
    /// Discogs said the token is not valid.
    Rejected,
    /// Discogs couldn't be asked (no connection, busy, a bad reply).
    CouldNotCheck,
    /// Discogs accepted it, but the password store refused it.
    StoreFailed,
}

/// Whether a Discogs token is saved. Never the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatus {
    pub saved: bool,
    pub store_available: bool,
}

/// What the switches say, and what the two services that need more than a
/// switch are missing.
pub fn status(
    reads: &ReadPool,
    credentials: &Credentials,
) -> Result<OnlineServicesStatus, DbError> {
    Ok(OnlineServicesStatus {
        music_brainz: is_opted_in(reads, Service::MusicBrainz)?,
        acoust_id: AcoustIdStatus {
            on: is_opted_in(reads, Service::AcoustId)?,
            key_missing: acoustid::key_missing(),
        },
        discogs: DiscogsStatus {
            on: is_opted_in(reads, Service::Discogs)?,
            token_saved: credentials.has(credentials::DISCOGS).unwrap_or(false),
            store_available: credentials.is_available(),
        },
        cover_art: is_opted_in(reads, Service::CoverArt)?,
    })
}

/// Switches `service` on or off.
pub fn set_service(writer: &Writer, service: OnlineService, on: bool) -> Result<(), DbError> {
    set_opt_in(writer, service.service(), on)
}

/// Saves a token if the Discogs switch is on and `check` says Discogs
/// accepts it. `check` is the one question put to Discogs, through the gate.
pub fn save_token(
    reads: &ReadPool,
    credentials: &Credentials,
    token: &str,
    check: impl FnOnce(&str) -> Result<(), ClientError>,
) -> Result<SaveTokenOutcome, DbError> {
    if !is_opted_in(reads, Service::Discogs)? {
        return Ok(SaveTokenOutcome::SwitchOff);
    }
    let token = token.trim();
    if token.is_empty() || token.chars().any(char::is_control) {
        return Ok(SaveTokenOutcome::Empty);
    }
    if !credentials.is_available() {
        return Ok(SaveTokenOutcome::StoreUnavailable);
    }
    match check(token) {
        Ok(()) => {}
        Err(ClientError::NoToken) => return Ok(SaveTokenOutcome::Empty),
        Err(ClientError::Unauthorized(_)) => return Ok(SaveTokenOutcome::Rejected),
        // The switch was turned off while the question waited its turn.
        Err(ClientError::Net(NetError::NotOptedIn(_))) => return Ok(SaveTokenOutcome::SwitchOff),
        Err(_) => return Ok(SaveTokenOutcome::CouldNotCheck),
    }
    // The check can wait a long time for its turn: look at the switch again
    // right before storing. (A switch turned off between this read and the
    // write below is still not caught: they are not one transaction.)
    if !is_opted_in(reads, Service::Discogs)? {
        return Ok(SaveTokenOutcome::SwitchOff);
    }
    Ok(match credentials.set(credentials::DISCOGS, token) {
        Ok(()) => SaveTokenOutcome::Saved,
        Err(_) => SaveTokenOutcome::StoreFailed,
    })
}

/// Whether a token is saved. A store that fails to answer counts as none.
pub fn token_status(credentials: &Credentials) -> TokenStatus {
    TokenStatus {
        saved: credentials.has(credentials::DISCOGS).unwrap_or(false),
        store_available: credentials.is_available(),
    }
}

/// Where the lookup services stand: each switch, AcoustID's missing key and
/// whether a Discogs token is saved.
#[tauri::command]
#[specta::specta]
pub async fn online_services_status(
    reads: State<'_, ReadPool>,
    credentials: State<'_, Credentials>,
) -> Result<OnlineServicesStatus, IpcError> {
    Ok(status(&reads, &credentials)?)
}

/// Switches one lookup service on or off. Nothing is sent while it is off.
#[tauri::command]
#[specta::specta]
pub async fn set_online_service(
    writer: State<'_, Writer>,
    service: OnlineService,
    on: bool,
) -> Result<(), IpcError> {
    set_service(&writer, service, on)?;
    Ok(())
}

/// Asks Discogs whether the token works and, if so, keeps it in the password
/// store. Needs the Discogs switch on.
#[tauri::command]
#[specta::specta]
pub async fn save_discogs_token(
    reads: State<'_, ReadPool>,
    net: State<'_, Net>,
    credentials: State<'_, Credentials>,
    token: String,
) -> Result<SaveTokenOutcome, IpcError> {
    let (reads, net, credentials) = ((*reads).clone(), (*net).clone(), (*credentials).clone());
    // The check blocks until the reply (and its turn at the rate limit).
    tauri::async_runtime::spawn_blocking(move || {
        save_token(&reads, &credentials, &token, |token| {
            discogs::check_token(&net, token)
        })
    })
    .await
    .map_err(|_| IpcError::new(ErrorKind::Internal))?
    .map_err(IpcError::from)
}

/// Whether a Discogs token is saved. The token itself never leaves the
/// password store.
#[tauri::command]
#[specta::specta]
pub async fn discogs_token_status(
    credentials: State<'_, Credentials>,
) -> Result<TokenStatus, IpcError> {
    Ok(token_status(&credentials))
}

/// Forgets the saved Discogs token (fine if there is none) and says whether
/// one is still saved, which is so only if the password store refused.
#[tauri::command]
#[specta::specta]
pub async fn remove_discogs_token(
    credentials: State<'_, Credentials>,
) -> Result<TokenStatus, IpcError> {
    let _ = credentials.clear(credentials::DISCOGS);
    Ok(token_status(&credentials))
}

#[cfg(test)]
mod tests;
