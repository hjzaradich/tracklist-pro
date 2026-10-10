//! Keeping up with drives that come and go (1aB-9, ROADMAP 1.1 "Offline
//! volumes").
//!
//! When Windows says a volume arrived or left, the app looks at its volumes
//! again ([`crate::paths::devices_changed`]), notes where each known volume
//! is now in the `volume` table, and tells the frontend, which reloads the
//! music folders: one whose drive is gone shows greyed out. Nothing about
//! its files changes; they're offline, not missing.
//!
//! The clone rule ([`crate::volume::tell_clones_apart`]) needs what the
//! library remembers about each volume. It's read once at startup and kept
//! here, and read again after anything changes a `volume` row.

use std::sync::{OnceLock, PoisonError, RwLock};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::db::{DbError, Writer};
use crate::volume::{Remembered, Volume, VolumeId};

/// What the library remembers about each volume, for the clone rule.
/// `None` until it has been read.
static REMEMBERED: RwLock<Option<Vec<Remembered>>> = RwLock::new(None);

/// Where to read it again if the first read failed.
static DATABASE: OnceLock<Writer> = OnceLock::new();

/// Drives came or went: the music folders' `online` may have changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct VolumesChanged;

/// What the `volume` table remembers: each volume's identity, last GUID
/// and label. Rows whose identity doesn't read back are left out.
pub fn remembered(conn: &Connection) -> rusqlite::Result<Vec<Remembered>> {
    let mut stmt = conn.prepare("SELECT identity, guid, label FROM volume ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (identity, guid, label) = row?;
        if let Ok(id) = VolumeId::from_stored(identity) {
            out.push(Remembered { id, guid, label });
        }
    }
    Ok(out)
}

/// Reads what the library remembers again, for the next volume lookups.
pub(crate) fn remember(conn: &Connection) -> rusqlite::Result<()> {
    let now = remembered(conn)?;
    *REMEMBERED.write().unwrap_or_else(PoisonError::into_inner) = Some(now);
    Ok(())
}

/// What the library remembered at the last [`remember`]. If it has never
/// been read (the startup read failed), it's read now; until a read
/// succeeds, clones are told apart without it.
pub(crate) fn remembered_now() -> Vec<Remembered> {
    let known = || {
        REMEMBERED
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    };
    if let Some(now) = known() {
        return now;
    }
    if let Some(writer) = DATABASE.get() {
        let _ = writer.call(|c| remember(c));
    }
    known().unwrap_or_default()
}

/// Notes where each volume the library knows is mounted now: its mount
/// path, label and when it was seen, and its GUID if none is remembered
/// yet. Returns how many rows it updated. Volumes the library doesn't
/// know aren't added.
///
/// `mounted` comes named by the clone rule, so a backup the library knows
/// arrives under its own `serial=…+guid={…}` identity and updates its own
/// row. A remembered GUID is never replaced: a backup the library has
/// never told apart, plugged in alone, answers to the original's plain
/// serial, and taking its GUID would make it the original the next time
/// both are plugged in (and the original's newer files would then look
/// missing).
pub fn note_mounted(
    conn: &mut Connection,
    mounted: &[(Volume, Option<String>)],
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let mut updated = 0;
    {
        let mut update = tx.prepare_cached(
            "UPDATE volume SET
                 label = ?2,
                 guid = coalesce(guid, ?3),
                 last_mount_path = ?4,
                 last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE identity = ?1",
        )?;
        for (volume, guid) in mounted {
            updated += update.execute((
                volume.id.as_str(),
                &volume.label,
                guid.as_deref(),
                super::display_path(&volume.mount_path),
            ))?;
        }
    }
    tx.commit()?;
    Ok(updated)
}

/// After drives came or went: looks at the volumes again, notes where the
/// known ones are, and rereads what the library remembers.
pub fn devices_changed(writer: &Writer) -> Result<(), DbError> {
    // Only Windows has volumes to look at again (ROADMAP §1.1): elsewhere
    // none is ever mounted (`mounted_now`), and there's nothing to tell.
    #[cfg(windows)]
    crate::paths::devices_changed();
    let mounted = mounted_now();
    writer.call(move |c| {
        note_mounted(c, &mounted)?;
        remember(c)
    })
}

#[cfg(windows)]
fn mounted_now() -> Vec<(Volume, Option<String>)> {
    super::system_volumes().mounted_with_guids()
}

#[cfg(not(windows))]
fn mounted_now() -> Vec<(Volume, Option<String>)> {
    Vec::new()
}

/// At startup, before the job queue, so the first scan already tells
/// clones apart: reads what the library remembers. Then, on a thread of its
/// own, notes where the known volumes are now and starts listening for
/// drives coming and going; each change reaches the frontend as
/// [`VolumesChanged`].
///
/// Nothing here can stop the app starting. If any of it fails, the app
/// still sees drives as they are whenever it next looks (every scan and
/// folder list looks afresh); only a job already running would keep an old
/// view until the next device change.
pub fn start<R: Runtime>(app: &AppHandle<R>, writer: &Writer) {
    if let Err(e) = writer.call(|c| remember(c)) {
        // Read again at the next lookup ([`remembered_now`]).
        eprintln!("volumes: couldn't read the remembered volumes at startup: {e}");
    }
    // Tests build many apps in one process, so they don't each start a
    // watch: they drive it (volume::devices) and the refresh
    // (`devices_changed`) directly.
    #[cfg(all(windows, not(test)))]
    {
        let _ = DATABASE.set(writer.clone());
        let (app, writer) = (app.clone(), writer.clone());
        let _ = std::thread::Builder::new()
            .name("volumes-start".into())
            .spawn(move || {
                let _ = devices_changed(&writer);
                watch(app, writer);
            });
    }
    #[cfg(any(not(windows), test))]
    let _ = app;
}

/// Listens for drives coming and going. Every volume message makes every
/// [`crate::paths::SystemVolumes`] look again before its next answer, at
/// once; the database refresh and the frontend's event follow once the
/// burst settles.
#[cfg(all(windows, not(test)))]
fn watch<R: Runtime>(app: AppHandle<R>, writer: Writer) {
    use tauri_specta::Event;
    let _ = crate::volume::devices::watch(crate::paths::devices_changed, move || {
        // A failed refresh keeps the last one; the next change retries.
        let _ = devices_changed(&writer);
        // Only fails if the app is closing.
        let _ = VolumesChanged.emit(&app);
    });
}
