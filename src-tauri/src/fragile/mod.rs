//! Fragile locations (1aD-7, ROADMAP §1.3): where a linked Library track's
//! file lives somewhere it could easily disappear from. A track depends on
//! its file, so the app warns, with the reason, when the file is in
//!
//! - the user's Downloads folder (Windows' known folder, wherever it was
//!   moved to, not a folder named "Downloads");
//! - a temp folder (`%TEMP%`, `%TMP%`, `Windows\Temp`);
//! - an external drive (`volume.kind = 'external'`);
//! - a network drive (`volume.kind = 'network'`).
//!
//! [`reason`] is the pure rule; [`fragile_reasons`] answers it for files in
//! the database. Neither touches a file.
//!
//! **Known limit (0B-10):** an NVMe drive in a Thunderbolt enclosure
//! reports itself as internal, so it is never flagged as external. The app
//! doesn't try to guess otherwise.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::paths::Volumes;
use crate::volume::{VolumeId, VolumeKind};

#[cfg(test)]
mod tests;

/// Why a file's location is fragile. The Library shows each with its
/// reason (the wording lives in the locale files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FragileReason {
    Downloads,
    Temp,
    External,
    Network,
}

/// The folders that make a location fragile, besides the drive's kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FragileDirs {
    pub downloads: Option<PathBuf>,
    pub temp: Vec<PathBuf>,
}

impl FragileDirs {
    /// This PC's Downloads folder and temp folders, each as the disk names
    /// it (Windows hands out 8.3 short names, like `USERNA~1`, for
    /// `%TEMP%`). Asks the disk, which can be slow for a folder redirected
    /// to a share: call it once, off the UI thread.
    pub fn system() -> FragileDirs {
        let mut temp = Vec::new();
        for var in ["TEMP", "TMP"] {
            if let Some(dir) = std::env::var_os(var) {
                temp.push(PathBuf::from(dir));
            }
        }
        if let Some(root) = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir")) {
            temp.push(Path::new(&root).join("Temp"));
        }
        let downloads = dirs::download_dir();
        FragileDirs {
            downloads: downloads.map(longest),
            temp: temp.into_iter().map(longest).collect(),
        }
    }
}

/// `dir` and, if the disk knows it by a longer name, that name.
fn longest(dir: PathBuf) -> PathBuf {
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

/// Why `path` (absolute) on a volume of `kind` is fragile, if it is. A
/// location in Downloads or a temp folder is reported as that, before the
/// drive's kind.
pub fn reason(path: &Path, kind: VolumeKind, dirs: &FragileDirs) -> Option<FragileReason> {
    if dirs.downloads.as_deref().is_some_and(|d| inside(path, d)) {
        return Some(FragileReason::Downloads);
    }
    if dirs.temp.iter().any(|d| inside(path, d)) {
        return Some(FragileReason::Temp);
    }
    match kind {
        VolumeKind::External => Some(FragileReason::External),
        VolumeKind::Network => Some(FragileReason::Network),
        VolumeKind::Internal => None,
    }
}

/// Whether `path` is `dir` or below it, compared the way Windows compares
/// names: letter case ignored, a `\\?\` prefix and the separator style not
/// mattering. A folder with fewer than two parts (a drive root, from a
/// `TEMP` set to `C:\`) holds nothing special and matches nothing.
fn inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (parts(path), parts(dir));
    dir.len() >= 2 && path.len() >= dir.len() && path[..dir.len()] == dir[..]
}

/// `text` without `prefix`, which is matched ignoring ASCII letter case.
fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// The name parts of a path, uppercased. Plain string work, so Windows
/// paths read the same on every platform.
fn parts(path: &Path) -> Vec<String> {
    let text = path.to_string_lossy();
    let text = match strip_prefix_ignoring_case(&text, r"\\?\UNC\") {
        Some(unc) => unc,
        None => text.strip_prefix(r"\\?\").unwrap_or(&text),
    };
    text.split(['\\', '/'])
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_uppercase)
        .collect()
}

/// Where a file is, as the database stores it: what [`judge`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    file: i64,
    kind: VolumeKind,
    identity: String,
    last_mount: Option<String>,
    /// The music folder and the file's path in it, `/`-separated.
    folder: String,
    rel: String,
}

/// Reads where `files` are (one prepared query for all of them). Files
/// that aren't in the database are left out. A read only: [`judge`] can
/// follow with no database connection held.
pub fn locate(conn: &Connection, files: &[i64]) -> rusqlite::Result<Vec<Located>> {
    let mut stmt = conn.prepare(
        "SELECT v.kind, v.identity, v.last_mount_path, mf.rel_path, f.rel_path
         FROM file f
         JOIN music_folder mf ON mf.id = f.music_folder_id
         JOIN volume v ON v.id = mf.volume_id
         WHERE f.id = ?1",
    )?;
    let mut found = Vec::new();
    for &file in files {
        let row = stmt.query_row([file], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        });
        let (kind, identity, last_mount, folder, rel) = match row {
            Ok(row) => row,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e),
        };
        let kind = match kind.as_str() {
            "external" => VolumeKind::External,
            "network" => VolumeKind::Network,
            _ => VolumeKind::Internal,
        };
        found.push(Located {
            file,
            kind,
            identity,
            last_mount,
            folder,
            rel,
        });
    }
    Ok(found)
}

/// Why each located file is fragile; files that aren't are left out.
///
/// The path is the volume's mount point now (or where it was last mounted,
/// if it's offline) plus the music folder and file. A volume that was never
/// seen mounted can only be judged by its kind.
pub fn judge(
    located: &[Located],
    volumes: &impl Volumes,
    dirs: &FragileDirs,
) -> HashMap<i64, FragileReason> {
    let mut found = HashMap::new();
    for at in located {
        let mount = VolumeId::from_stored(at.identity.clone())
            .ok()
            .and_then(|id| volumes.mount_path(&id))
            .or_else(|| at.last_mount.clone().map(PathBuf::from));
        let why = match mount {
            Some(mut path) => {
                for part in at.folder.split('/').chain(at.rel.split('/')) {
                    if !part.is_empty() {
                        path.push(part);
                    }
                }
                reason(&path, at.kind, dirs)
            }
            None => reason(Path::new(""), at.kind, dirs),
        };
        if let Some(why) = why {
            found.insert(at.file, why);
        }
    }
    found
}

/// Why each of `files` is fragile: [`locate`] then [`judge`].
pub fn fragile_reasons(
    conn: &Connection,
    volumes: &impl Volumes,
    dirs: &FragileDirs,
    files: &[i64],
) -> rusqlite::Result<HashMap<i64, FragileReason>> {
    Ok(judge(&locate(conn, files)?, volumes, dirs))
}
