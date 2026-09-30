//! Relink: matching each rekordbox track to its file (1aC-3 to 1aC-5,
//! ROADMAP 1.2, §2, §5.3).
//!
//! rekordbox names a track's file by its `Location`, and those go stale
//! when a DJ moves files, changes computers or renames things. Relink finds
//! the file again, and records how in `rekordbox_track.relink_method` and
//! `relink_confidence`. A wrong match is worse than none: it attaches one
//! track's cues, grid and play counts to another track's file. So every
//! step below matches only when the evidence picks out exactly one file,
//! and anything else stays missing (with its rekordbox data intact) for
//! the later steps (1aD) or for the user.
//!
//! In order, each step over every row before the next step starts:
//!
//! 0. **A confirmed relink** (the `relink` table, keyed by the Location's
//!    match key) is re-applied first, over any automatic match, as long as
//!    its file is still present. The user said so; nothing overrides it.
//! 1. **Path** (`path`): the decoded Location still names a present file,
//!    compared by the NFC match key with letter case folded, so an NFD
//!    Location finds its NFC file and `e:/MUSIC` finds `E:\Music`. A drive
//!    that's plugged in is asked by where it's mounted now. Only if none
//!    holds the path is a drive that isn't plugged in asked, by where it
//!    was last mounted; its files stay present while it's away (1.1), and
//!    the DJ's rekordbox still points at them. If two files share the key
//!    (an NFC name and its NFD twin, two files on NTFS), the one spelled
//!    exactly like the Location wins, as on Windows; otherwise no match.
//!    - *False-match risk:* low. The path is what rekordbox itself opens.
//!      It can be wrong when the file at that path was replaced by other
//!      audio under the same name, which no path rule can see, or when a
//!      drive that isn't plugged in had the same letter as the drive the
//!      export meant: if two drives last mounted at that letter both hold
//!      the path, there's no match, and one drive with the file wins only
//!      when no drive plugged in holds it (confidence 0.9, not 1.0).
//! 2. **File name + duration** (`filename_duration`): a present file
//!    anywhere with the same name (NFC, letter case ignored) whose duration
//!    fits `TotalTime`. rekordbox truncates `TotalTime` to whole seconds
//!    (§5.3), so its own duration is in `[T, T+1)` s; allowing ±0.5 s, the
//!    file must last from `T·1000 − 500` ms up to, not including,
//!    `T·1000 + 1500` ms ([`duration_fits`]).
//!    - *False-match risk:* two different recordings with the same file
//!      name and durations within about 2 s: a clean and a dirty cut both
//!      named `01 Title.mp3` in two folders, or generic names
//!      (`Track 01.mp3`). Guarded by being one-to-one: the track must have
//!      exactly one fitting file, that file must fit no other unmatched
//!      track, no same-named file of unknown duration may exist (it could
//!      fit too), and a file another row already has is never taken. So
//!      one surviving copy of two rekordbox entries stays unmatched rather
//!      than attached to both.
//! 3. **Unique duration** (`unique_duration`): the only file of fitting
//!    duration in the track's **candidate set**, defined as the files in:
//!    - the folder the Location's own folder still names (a file renamed
//!      where it was), found the same way as step 1; and
//!    - every folder that the track's neighbours were matched into, where
//!      neighbours are rows whose Location is in the same rekordbox folder
//!      and "matched" means confirmed, or by a step that says where a
//!      file went (path, filename + duration; later fingerprint, gig
//!      stick, user). Duration-only and name-only guesses aren't evidence,
//!      so guesses never chain.
//!
//!    Why this set: a renamed file is usually still in its folder, and a
//!    moved one usually moved with its album or crate. Anywhere else,
//!    duration alone means nothing: in a library of thousands, dozens of
//!    files share any two-second window.
//!    - *False-match risk:* the highest of the three. When the track's own
//!      file was deleted, another file in the same folder can happen to
//!      fit: a clean/dirty or radio/extended cut can differ by under 2 s.
//!      Guarded by: uniqueness counted over every present file in the set,
//!      including files other rows already have (so a claimed look-alike
//!      blocks the match instead of being skipped over); the one fitting
//!      file must be unclaimed and fit no other unmatched track; the set
//!      must be at most [`MAX_CANDIDATES`] files; and uniqueness must be
//!      provable, so if any file in the set has an unknown duration (not
//!      read yet, online-only, or unreadable), there's no match until a
//!      later run. A set of more than 50 files skips step 3 for that track
//!      (it isn't cut down to 50).
//!
//!    **A second signal decides whether it's trusted.** The match is
//!    accepted (confidence 0.8) only when one of the file's title tags
//!    agrees with rekordbox's `Name` ([`rules::title_key`]: NFKC,
//!    lowercase, featuring credits dropped, punctuation and spacing folded,
//!    bracket contents kept, so `(Clean)` never agrees with `(Dirty)`).
//!    Otherwise, and when the file has no title tag, it's stored as
//!    **probable** (`relink_probable = 1`, confidence 0.4), like a
//!    filename-only match: its file is taken, but no rekordbox data is
//!    attached until the user confirms it in Review. The remaining risk,
//!    a deleted file plus one look-alike cut in its folder, lands there.
//!
//! A file of unknown duration blocks step 2 too: a same-named file that's
//! online-only (never read unless the user opts in) or not read yet makes
//! the name ambiguous, and the track falls through to step 3.
//!
//! **Never on size** (§5.3): rekordbox rewrites tags, so sizes change and
//! the XML's `Size` can be stale. Nothing here reads it.
//!
//! **Streaming entries** (`soundcloud:…`) are never matched, not even by a
//! confirmed relink. Rows already matched before a run keep their match
//! (only a confirmed relink replaces one), and their files count as taken;
//! a fresh rekordbox read starts every row unmatched. Running again with
//! nothing changed changes nothing.
//!
//! Relink reads only the database: no file is opened. It runs as a job
//! ([`job`]), queued after every rekordbox read, and again when asked.

pub mod job;
pub mod rules;

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::paths::Volumes;
use crate::volume::VolumeId;

pub use job::{relink_job, relink_rekordbox_tracks, relinker, request, Relinker};
pub use rules::{duration_fits, Method, MAX_CANDIDATES};

/// Where each known volume is mounted right now, by its stored identity:
/// the mount point as the OS gives it (`E:\`, `\\?\E:\`,
/// `\\?\UNC\server\share\`). A volume not listed isn't plugged in.
#[derive(Debug, Clone, Default)]
pub struct Mounted(HashMap<String, String>);

impl Mounted {
    /// From `(identity, mount point)` pairs.
    pub fn new<I, S, T>(pairs: I) -> Mounted
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: Into<String>,
    {
        Mounted(
            pairs
                .into_iter()
                .map(|(id, mount)| (id.into(), mount.into()))
                .collect(),
        )
    }

    /// Asks `volumes` where each of `identities` is mounted now.
    pub fn ask(identities: &[String], volumes: &impl Volumes) -> Mounted {
        Mounted(
            identities
                .iter()
                .filter_map(|identity| {
                    let id = VolumeId::from_stored(identity.clone()).ok()?;
                    let mount = volumes.mount_path(&id)?;
                    Some((identity.clone(), mount.to_string_lossy().into_owned()))
                })
                .collect(),
        )
    }
}

/// Every volume identity the library knows.
pub fn identities(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT identity FROM volume ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect()
}

/// A mount point as a `/`-separated path without a trailing `/`: `E:`,
/// `C:/mnt/usb`, `//server/share`. `None` if it's neither a drive nor a
/// network path.
fn mount_text(mount: &str) -> Option<String> {
    let plain = if let Some(unc) = mount.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = mount.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        mount.to_owned()
    };
    let text = plain.replace('\\', "/");
    let text = text.trim_end_matches('/');
    let b = text.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let network = text.len() > 2 && text.starts_with("//") && !text[2..].starts_with('/');
    (drive || network).then(|| text.to_owned())
}

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Matched in this run, by step.
    pub path: u32,
    pub filename_duration: u32,
    /// Accepted: a title tag agrees.
    pub unique_duration: u32,
    /// Stored as probable, waiting for the user to confirm.
    pub probable: u32,
    /// Confirmed relinks re-applied where the row didn't already have them.
    pub confirmed: u32,
    /// Rows already matched before this run and left alone.
    pub kept: u32,
    /// Streaming entries, never matched.
    pub streaming: u32,
    /// Rows still without a file.
    pub missing: u32,
}

/// Matches every unmatched `rekordbox_track` row it can, in one
/// transaction. See the module docs for the rules.
pub fn relink(conn: &mut Connection, mounted: &Mounted) -> rusqlite::Result<Summary> {
    let tx = conn.transaction()?;
    let input = load(&tx, mounted)?;
    let plan = rules::plan(&input);
    let mut summary = Summary {
        kept: count(plan.kept),
        streaming: count(plan.streaming),
        missing: count(plan.missing),
        ..Summary::default()
    };
    {
        let mut update = tx.prepare(
            "UPDATE rekordbox_track
             SET file_id = ?2, relink_method = ?3, relink_confidence = ?4,
                 relink_probable = ?5
             WHERE id = ?1",
        )?;
        for d in &plan.decisions {
            update.execute(params![
                d.track,
                d.file,
                d.method.as_str(),
                d.confidence,
                d.probable,
            ])?;
            let n = match (d.confirmed, d.probable, d.method) {
                (true, _, _) => &mut summary.confirmed,
                (false, true, _) => &mut summary.probable,
                (false, false, Method::Path) => &mut summary.path,
                (false, false, Method::FilenameDuration) => &mut summary.filename_duration,
                (false, false, _) => &mut summary.unique_duration,
            };
            *n += 1;
        }
    }
    tx.commit()?;
    Ok(summary)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Reads what the rules look at.
fn load(conn: &Connection, mounted: &Mounted) -> rusqlite::Result<rules::Input> {
    // Each music folder's path: under its volume's mount point now, or
    // where the volume was last mounted if it isn't plugged in.
    let mut folders: HashMap<i64, (Option<String>, bool)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT m.id, m.rel_path, v.identity, v.last_mount_path
             FROM music_folder m JOIN volume v ON v.id = m.volume_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        for row in rows {
            let (id, rel, identity, last) = row?;
            let (mount, online) = match mounted.0.get(&identity) {
                Some(now) => (mount_text(now), true),
                None => (last.as_deref().and_then(mount_text), false),
            };
            let base = mount.map(|m| {
                if rel.is_empty() {
                    m
                } else {
                    format!("{m}/{rel}")
                }
            });
            folders.insert(id, (base, online));
        }
    }

    let mut files = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, music_folder_id, rel_path, duration_ms, raw_tags
             FROM file WHERE present = 1",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        for row in rows {
            let (id, folder, rel, duration_ms, raw_tags) = row?;
            let (base, online) = folders.get(&folder).cloned().unwrap_or((None, false));
            let (parent, name) = match rel.rsplit_once('/') {
                Some((parent, name)) => (parent.to_owned(), name.to_owned()),
                None => (String::new(), rel.clone()),
            };
            files.push(rules::File {
                id,
                folder,
                parent,
                name,
                path: base.map(|b| format!("{b}/{rel}")),
                online,
                duration_ms,
                titles: raw_tags.as_deref().map(titles).unwrap_or_default(),
            });
        }
    }

    let mut tracks = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, location, location_key, json_extract(attributes, '$.TotalTime'),
                    json_extract(attributes, '$.Name'),
                    file_id, relink_method, relink_confidence, relink_probable
             FROM rekordbox_track ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<f64>>(7)?,
                r.get::<_, bool>(8)?,
            ))
        })?;
        for row in rows {
            let (id, location, key, total, name, file, method, confidence, probable) = row?;
            let total_s = total
                .as_deref()
                .and_then(crate::rekordbox::attrs::digits::<u32>)
                .filter(|&t| t > 0);
            // The table allows only the methods `Method` knows; should a
            // newer build add one, the row keeps its file (it's taken).
            let current = file.map(|file| rules::Current {
                file,
                method: method
                    .as_deref()
                    .and_then(Method::parse)
                    .unwrap_or(Method::User),
                confidence,
                probable,
            });
            tracks.push(rules::Track {
                id,
                key,
                location: crate::rekordbox::location::decode(&location).ok(),
                total_s,
                name: name.as_deref().map(rules::title_key).unwrap_or_default(),
                current,
            });
        }
    }

    let mut confirmed = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT location_key, file_id, method FROM relink")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (key, file, method) = row?;
            if let Some(method) = Method::parse(&method) {
                confirmed.insert(key, rules::Confirmed { file, method });
            }
        }
    }

    Ok(rules::Input {
        files,
        tracks,
        confirmed,
    })
}

/// The title tags in a file's `raw_tags` (one array of `{key, value}` per
/// tag block type), each as a [`rules::title_key`]. Every block's own
/// title field counts: ID3v2 `TIT2`, ID3v1 `title`, APE and Vorbis
/// `TITLE` (any case), MP4 `©nam`, RIFF `INAM`, AIFF `NAME`.
fn titles(raw_tags: &str) -> Vec<String> {
    let Ok(serde_json::Value::Object(blocks)) = serde_json::from_str(raw_tags) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (block, items) in &blocks {
        let is_title = |key: &str| match block.as_str() {
            "id3v2" => key == "TIT2",
            "id3v1" => key == "title",
            "ape" | "vorbis_comments" => key.eq_ignore_ascii_case("title"),
            "mp4_ilst" => key == "\u{a9}nam",
            "riff_info" => key == "INAM",
            "aiff_text" => key == "NAME",
            _ => false,
        };
        for item in items.as_array().into_iter().flatten() {
            let key = item.get("key").and_then(|k| k.as_str()).unwrap_or("");
            let text = item
                .get("value")
                .and_then(|v| v.get("text"))
                .and_then(|t| t.as_str());
            if let (true, Some(text)) = (is_title(key), text) {
                let title = rules::title_key(text);
                if !title.is_empty() {
                    out.push(title);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
