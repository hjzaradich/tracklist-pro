//! Matching a play's path to a track, by the same path key relink's first
//! step uses ([`crate::relink::rules::path_key`]: NFC, letter case folded as
//! NTFS folds it), against the present files the scan indexed. A drive
//! that's plugged in is asked first, then drives that aren't, by where they
//! were last mounted ([`crate::relink::volume_mounts`]). A path that names no
//! file, or files of more than one track, matches nothing: a wrong match is
//! worse than none, and the play is kept either way.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::relink::rules::path_key;
use crate::relink::{volume_mounts, Mounted};

/// The present files' tracks, by path key, plugged-in drives and others apart.
pub struct PathIndex {
    online: HashMap<String, Vec<Option<i64>>>,
    offline: HashMap<String, Vec<Option<i64>>>,
}

impl PathIndex {
    pub fn load(conn: &Connection, mounted: &Mounted) -> rusqlite::Result<PathIndex> {
        let mounts: HashMap<i64, (Option<String>, bool)> = volume_mounts(conn, mounted)?
            .into_iter()
            .map(|v| (v.id, (v.mount, v.online)))
            .collect();
        let mut index = PathIndex {
            online: HashMap::new(),
            offline: HashMap::new(),
        };
        let mut stmt = conn.prepare(
            "SELECT mf.volume_id, mf.rel_path, f.rel_path, rf.recording_id
             FROM file f
             JOIN music_folder mf ON mf.id = f.music_folder_id
             LEFT JOIN recording_file rf ON rf.file_id = f.id
             WHERE f.present",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for row in rows {
            let (volume, folder, rel, recording) = row?;
            let Some((Some(mount), online)) = mounts.get(&volume).cloned() else {
                continue;
            };
            let path = [mount.as_str(), folder.as_str(), rel.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            let map = if online {
                &mut index.online
            } else {
                &mut index.offline
            };
            map.entry(path_key(&path)).or_default().push(recording);
        }
        Ok(index)
    }

    /// The track the play's `path` names. A relative path is taken from
    /// `base`, the folder of the playlist file (`/`-separated), if known.
    pub fn recording_for(&self, path: &str, base: Option<&str>) -> Option<i64> {
        let full = absolute(path, base)?;
        let key = path_key(&full);
        for map in [&self.online, &self.offline] {
            if let Some(found) = map.get(&key) {
                let first = found.first().copied().flatten()?;
                return found.iter().all(|r| *r == Some(first)).then_some(first);
            }
        }
        None
    }
}

/// `path` with `/` separators, made absolute against `base` when it is
/// relative, with `.` and `..` worked out. `None` when it is relative and
/// there is no base, or `..` climbs past the top.
pub fn absolute(path: &str, base: Option<&str>) -> Option<String> {
    let path = path.replace('\\', "/");
    let rooted = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    let joined = if rooted {
        path
    } else {
        format!("{}/{path}", base?.replace('\\', "/").trim_end_matches('/'))
    };
    // `//server/share` keeps its two slashes.
    let (head, rest) = match joined.strip_prefix("//") {
        Some(rest) => ("//", rest),
        None => match joined.strip_prefix('/') {
            Some(rest) => ("/", rest),
            None => ("", joined.as_str()),
        },
    };
    // How many leading parts `..` may never remove: the server and share of
    // a `//` path, the drive (`E:`) of a drive path; a plain `/` has none.
    let floor = match head {
        "//" => 2,
        "" => 1,
        _ => 0,
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.len() <= floor {
                    return None;
                }
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    Some(format!("{head}{}", parts.join("/")))
}
