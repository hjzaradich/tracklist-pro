//! Update checks (ROADMAP 4.5): the app asks GitHub, through the network
//! gate, whether a newer version exists, and says so. It checks and tells;
//! nothing is downloaded or run.
//!
//! Off until the user says yes: the first run asks once, Settings has the
//! switch. The service's opt-in lives in `service_optin` like every
//! service's (the gate reads it); two `setting` rows hold the rest:
//!
//! - `update_check_asked`: the question has been answered, yes or no, so the
//!   first-run step never shows again. It does not depend on the Library.
//! - `update_check_result`: when the last answer came and the version it
//!   named (`null` for "no newer release"). A check that couldn't reach
//!   GitHub stores nothing, so the next start tries again.
//!
//! Whether a version is newer is judged when the status is read, against the
//! running app, so a stored answer goes stale on its own after an update.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::IpcError;
use crate::net::services::update::{is_newer, latest, Release};
use crate::net::services::ClientError;
use crate::net::{is_opted_in, set_opt_in, Net, Service};

#[cfg(test)]
mod tests;

const ASKED: &str = "update_check_asked";
const RESULT: &str = "update_check_result";

/// The running app's version.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A newer version, as the last check found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    /// The release page, for the user to copy. The app opens no links.
    pub url: String,
}

/// Where update checks stand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The first-run question has been answered.
    pub asked: bool,
    /// Update checks are on.
    pub enabled: bool,
    /// The running version.
    pub current: String,
    /// A newer version, when the last check found one.
    pub available: Option<Available>,
    /// When the last check got an answer (UTC, ISO 8601).
    pub checked_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    checked_at: String,
    version: Option<String>,
    url: Option<String>,
}

fn read_stored(conn: &Connection) -> rusqlite::Result<Option<Stored>> {
    let json: Option<String> = conn
        .query_row("SELECT value FROM setting WHERE key = ?1", [RESULT], |r| {
            r.get(0)
        })
        .optional()?;
    // A value this version can't read is no answer.
    Ok(json.and_then(|json| serde_json::from_str(&json).ok()))
}

fn read_asked(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT json_type(value) = 'true' FROM setting WHERE key = ?1",
            [ASKED],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

fn write_setting(conn: &Connection, key: &str, json: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, json(?2))
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (key, json),
    )?;
    Ok(())
}

/// Whether the last answer is a day old or more, or there is none.
fn due(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(match read_stored(conn)? {
        None => true,
        Some(stored) => conn.query_row(
            "SELECT COALESCE(datetime(?1) <= datetime('now', '-1 day'), 1)",
            [stored.checked_at],
            |r| r.get(0),
        )?,
    })
}

fn record(conn: &Connection, release: Option<&Release>) -> rusqlite::Result<()> {
    let at: String = conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
        r.get(0)
    })?;
    let stored = Stored {
        checked_at: at,
        version: release.map(|r| r.version.clone()),
        url: release.map(|r| r.url.clone()),
    };
    write_setting(
        conn,
        RESULT,
        &serde_json::to_string(&stored).expect("plain strings serialize"),
    )
}

/// The status for a running version of `current`.
fn status_for(conn: &Connection, current: &str) -> rusqlite::Result<UpdateStatus> {
    let enabled: Option<i64> = conn
        .query_row(
            "SELECT enabled FROM service_optin WHERE service = ?1",
            [Service::UpdateCheck.id()],
            |r| r.get(0),
        )
        .optional()?;
    let stored = read_stored(conn)?;
    let available = stored.as_ref().and_then(|s| match (&s.version, &s.url) {
        (Some(version), Some(url)) if is_newer(version, current) => Some(Available {
            version: version.clone(),
            url: url.clone(),
        }),
        _ => None,
    });
    Ok(UpdateStatus {
        asked: read_asked(conn)?,
        enabled: enabled == Some(1),
        current: current.to_owned(),
        available,
        checked_at: stored.map(|s| s.checked_at),
    })
}

/// Checks if update checks are on and (unless `force`) the last answer is a
/// day old, then returns the status. `fetch` asks GitHub. A check that
/// fails, or is refused because the user opted out meanwhile, leaves what
/// is stored as it was.
pub(crate) fn check_with(
    reads: &ReadPool,
    writer: &Writer,
    force: bool,
    fetch: impl FnOnce() -> Result<Option<Release>, ClientError>,
) -> Result<UpdateStatus, DbError> {
    let wanted = is_opted_in(reads, Service::UpdateCheck)? && (force || reads.read(due)?);
    if wanted {
        if let Ok(release) = fetch() {
            writer.call(move |c| record(c, release.as_ref()))?;
        }
    }
    reads.read(|c| status_for(c, CURRENT))
}

/// Where update checks stand.
#[tauri::command]
#[specta::specta]
pub async fn update_status(reads: State<'_, ReadPool>) -> Result<UpdateStatus, IpcError> {
    Ok(reads.read(|c| status_for(c, CURRENT))?)
}

/// Answers the question and sets the switch: turns update checks on or off
/// and records that the user has been asked.
#[tauri::command]
#[specta::specta]
pub async fn set_update_checks(writer: State<'_, Writer>, on: bool) -> Result<(), IpcError> {
    set_opt_in(&writer, Service::UpdateCheck, on)?;
    writer.call(|c| write_setting(c, ASKED, "true"))?;
    Ok(())
}

/// Checks for a newer version, if update checks are on: always with `force`
/// ("Check now"), otherwise only when the last answer is a day old (the
/// start of the app). Returns the status either way; a failed check is not an
/// error.
#[tauri::command]
#[specta::specta]
pub async fn check_for_updates(
    reads: State<'_, ReadPool>,
    writer: State<'_, Writer>,
    net: State<'_, Net>,
    force: bool,
) -> Result<UpdateStatus, IpcError> {
    let (reads, writer, net) = ((*reads).clone(), (*writer).clone(), (*net).clone());
    // The request blocks until the reply (and its turn at the rate limit).
    let status = tauri::async_runtime::spawn_blocking(move || {
        check_with(&reads, &writer, force, || latest(&net))
    })
    .await
    .map_err(|_| IpcError::new(crate::ipc::ErrorKind::Internal))?;
    Ok(status?)
}
