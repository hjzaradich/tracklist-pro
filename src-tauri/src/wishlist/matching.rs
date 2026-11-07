//! What a wish item comes to against the music the user has (ROADMAP 2.3).
//! Worked out when read, never stored.
//!
//! One [`Matcher`] serves a whole run: it reads the owned tracks once, in
//! one pass, and each item is then looked up in a map. Nothing is scanned
//! per item, so a run stays fast at 100,000 tracks.
//!
//! - **Owned** is a track with a file that is on disk and not hidden. The
//!   reason says whether the track is in the Library or only in All music.
//! - An ISRC (read from the files' stored tags) that an owned file carries
//!   wins over any name match.
//! - Otherwise the artist and the base title find the candidate tracks, and
//!   the version parser ([`versions::compare`]) says how the item's name
//!   stands to each: the same version is owned, another cut is owned (other
//!   cut), another rework is only a hint. The best candidate decides: same
//!   version, then cut, then rework; a Library track before one only in All
//!   music; then the lowest id.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use specta::Type;

use super::parse::WishTrack;
use crate::send_values::shown_recordings;
use crate::suggest::Reason;
use crate::versions::{self, Outcome};

/// The i18n key of each reason. Parameter `have` (the owned track's title)
/// goes with every one but `MISSING`.
pub mod keys {
    pub const OWNED_LIBRARY: &str = "wishlist:reason.ownedLibrary";
    pub const OWNED_ALL_MUSIC: &str = "wishlist:reason.ownedAllMusic";
    pub const OWNED_ISRC_LIBRARY: &str = "wishlist:reason.ownedIsrcLibrary";
    pub const OWNED_ISRC_ALL_MUSIC: &str = "wishlist:reason.ownedIsrcAllMusic";
    pub const OTHER_CUT_LIBRARY: &str = "wishlist:reason.otherCutLibrary";
    pub const OTHER_CUT_ALL_MUSIC: &str = "wishlist:reason.otherCutAllMusic";
    pub const REWORK: &str = "wishlist:reason.rework";
    pub const MISSING: &str = "wishlist:reason.missing";
}

/// What an item comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WishMatchKind {
    /// A track you have.
    Owned,
    /// Another cut of it; counts as owned.
    OwnedOtherCut,
    /// A rework of it: a hint, not owned.
    Rework,
    Missing,
}

/// An item's result with its reason.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WishMatch {
    pub kind: WishMatchKind,
    pub reason: Reason,
    /// The track it is about (the one owned, or the one reworked).
    pub recording_id: Option<i64>,
    /// That track is in the Library (not only in All music).
    pub in_library: bool,
}

/// The result for an item nothing matches.
pub fn missing() -> WishMatch {
    WishMatch {
        kind: WishMatchKind::Missing,
        reason: fact(keys::MISSING, None),
        recording_id: None,
        in_library: false,
    }
}

pub(super) fn fact(key: &str, have: Option<&str>) -> Reason {
    let reason = Reason::fact(key).unwrap_or_else(|e| panic!("bad reason key {key}: {e}"));
    match have {
        Some(have) => reason
            .with_text("have", have)
            .unwrap_or_else(|e| panic!("bad reason parameter: {e}")),
        None => reason,
    }
}

/// An ISRC as stored: twelve letters and digits, upper case; `None` for
/// anything else. Dashes and spaces ("US-ABC-12-34567") are dropped.
pub fn normalize_isrc(text: &str) -> Option<String> {
    let isrc: String = text
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .collect::<String>()
        .to_uppercase();
    (isrc.len() == 12 && isrc.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(isrc)
}

/// The ISRCs in a file's `raw_tags` (blocks of `{key, value: {text}}`
/// frames): ID3's `TSRC`, Vorbis and APE's `ISRC`, MP4's freeform `ISRC`.
/// Anything not shaped like that gives none.
pub(crate) fn isrcs_in(raw_tags: &str) -> Vec<String> {
    let Ok(Value::Object(blocks)) = serde_json::from_str::<Value>(raw_tags) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for frames in blocks.values().filter_map(Value::as_array) {
        for frame in frames {
            let key = frame.get("key").and_then(Value::as_str).unwrap_or("");
            let key = key.to_ascii_lowercase();
            if key != "tsrc" && key != "isrc" && !key.ends_with(":isrc") {
                continue;
            }
            let text = frame
                .get("value")
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            // ID3v2.4 separates several values with NUL.
            out.extend(text.split('\0').filter_map(normalize_isrc));
        }
    }
    out
}

/// A lookup key for a name; `None` for one with no letter or digit.
fn lookup_key(text: &str) -> Option<String> {
    let key = versions::title_key(text);
    key.chars().any(char::is_alphanumeric).then_some(key)
}

/// The keys of the artists an artist tag names, and of the tag as a whole.
pub(super) fn artist_keys(artist: &str) -> Vec<String> {
    let mut keys: Vec<String> = versions::parse_artists(artist)
        .iter()
        .filter_map(|credit| lookup_key(&credit.name))
        .collect();
    keys.extend(lookup_key(artist));
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Below this many tracks, work is not worth spreading over threads.
const PARALLEL_FROM: usize = 4096;

/// For each of `artists`, its [`artist_keys`] that are in `wanted` (all of
/// them for `None`), in order. The artist parser is most of what building
/// a matcher over a big library costs once titles are left to be read on
/// demand, so the artists are shared among the cores.
fn wanted_artist_keys(artists: &[&str], wanted: Option<&HashSet<String>>) -> Vec<Vec<String>> {
    let keys_of = |artist: &&str| -> Vec<String> {
        let mut keys = artist_keys(artist);
        keys.retain(|key| wanted.is_none_or(|w| w.contains(key)));
        keys
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    if artists.len() < PARALLEL_FROM || threads < 2 {
        return artists.iter().map(keys_of).collect();
    }
    let chunk = artists.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let parts: Vec<_> = artists
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(keys_of).collect::<Vec<_>>()))
            .collect();
        parts
            .into_iter()
            .flat_map(|part| part.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}
/// The keys the base title can be read as.
pub(super) fn title_keys(name: &versions::ParsedName) -> Vec<String> {
    let mut keys: Vec<String> = name
        .readings()
        .iter()
        .filter_map(|reading| lookup_key(&reading.base_title))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// A candidate's title read by the version parser, and the keys its base
/// title can be read as.
struct Read {
    parsed: versions::ParsedName,
    keys: Vec<String>,
}

pub(super) struct Candidate {
    pub(super) recording: i64,
    pub(super) in_library: bool,
    /// The title as the lists show it.
    pub(super) title: String,
    /// Read when an item first asks about the candidate: the parser costs
    /// most of a millisecond a title, and most tracks have no artist an
    /// item names (found at 100,000 tracks: building the matcher took a
    /// minute with every title read up front).
    read: OnceLock<Read>,
}

impl Candidate {
    fn read(&self) -> &Read {
        self.read.get_or_init(|| {
            let parsed = versions::parse_title(&self.title);
            let keys = title_keys(&parsed);
            Read { parsed, keys }
        })
    }
}

/// The item's title with its version, as a file's title would carry it.
pub(super) fn wish_name(wish: &WishTrack) -> String {
    match &wish.version {
        Some(version) => format!("{} ({version})", wish.title),
        None => wish.title.clone(),
    }
}

// How many matchers this thread has built (tests of the early exits).
#[cfg(test)]
thread_local! {
    pub(crate) static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct Matcher {
    pub(super) candidates: Vec<Candidate>,
    /// The candidates each artist key names, in order.
    pub(super) by_artist: HashMap<String, Vec<usize>>,
    by_recording: HashMap<i64, usize>,
    by_isrc: HashMap<String, i64>,
}

impl Matcher {
    /// Reads the owned tracks: one query for which they are, one for their
    /// names, one for the ISRCs of their files.
    pub fn build(conn: &Connection) -> rusqlite::Result<Matcher> {
        Matcher::build_among(conn, None, None)
    }

    /// [`Matcher::build`] for looking up `wishes` only, over `recordings` only
    /// (those of them that are owned) when given: what the Inbox tick-off
    /// asks about its new files (2.5). The files' tags are read for ISRCs
    /// only when a wish has one, and only those are kept; every answer for
    /// these wishes is the same.
    pub fn build_wanting<'a>(
        conn: &Connection,
        recordings: Option<&[i64]>,
        wishes: impl IntoIterator<Item = &'a WishTrack>,
    ) -> rusqlite::Result<Matcher> {
        let wishes: Vec<&WishTrack> = wishes.into_iter().collect();
        let isrcs: HashSet<String> = wishes
            .iter()
            .filter_map(|w| w.isrc.as_deref().and_then(normalize_isrc))
            .collect();
        let artists: HashSet<String> = wishes.iter().flat_map(|w| artist_keys(&w.artist)).collect();
        Matcher::build_among(conn, recordings, Some((isrcs, artists)))
    }

    /// `wanted`: the only ISRCs and artist keys worth keeping (`None`: all of
    /// them).
    fn build_among(
        conn: &Connection,
        among: Option<&[i64]>,
        wanted: Option<(HashSet<String>, HashSet<String>)>,
    ) -> rusqlite::Result<Matcher> {
        #[cfg(test)]
        BUILT.with(|n| n.set(n.get() + 1));
        let (wanted_isrcs, wanted_artists) = match wanted {
            Some((isrcs, artists)) => (Some(isrcs), Some(artists)),
            None => (None, None),
        };
        // NULL: every track.
        let among = among.map(|ids| serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()));
        let owned: Vec<(i64, bool)> = conn
            .prepare_cached(
                "SELECT rf.recording_id,
                        EXISTS (SELECT 1 FROM library_track lt
                                WHERE lt.recording_id = rf.recording_id)
                 FROM recording_file rf
                 JOIN file f ON f.id = rf.file_id
                 WHERE f.present = 1 AND f.hidden_reason IS NULL
                   AND (?1 IS NULL
                        OR rf.recording_id IN (SELECT value FROM json_each(?1)))
                 GROUP BY rf.recording_id
                 ORDER BY rf.recording_id",
            )?
            .query_map([&among], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let ids: Vec<i64> = owned.iter().map(|(id, _)| *id).collect();
        let mut names = if ids.is_empty() {
            HashMap::new()
        } else {
            shown_recordings(conn, &ids)?
        };
        let mut matcher = Matcher {
            candidates: Vec::with_capacity(owned.len()),
            by_artist: HashMap::new(),
            by_recording: HashMap::new(),
            by_isrc: HashMap::new(),
        };
        let mut shown: Vec<(i64, bool, crate::send_values::Shown)> =
            Vec::with_capacity(owned.len());
        for (recording, in_library) in owned {
            if let Some(name) = names.remove(&recording) {
                shown.push((recording, in_library, name));
            }
        }
        // Only the artists an item names can ever be asked for.
        let artists = wanted_artist_keys(
            &shown
                .iter()
                .map(|(_, _, s)| s.artist.as_str())
                .collect::<Vec<_>>(),
            wanted_artists.as_ref(),
        );
        for ((recording, in_library, name), keys) in shown.into_iter().zip(artists) {
            let at = matcher.candidates.len();
            for key in keys {
                matcher.by_artist.entry(key).or_default().push(at);
            }
            matcher.by_recording.insert(recording, at);
            matcher.candidates.push(Candidate {
                recording,
                in_library,
                title: name.title,
                read: OnceLock::new(),
            });
        }
        if wanted_isrcs.as_ref().is_some_and(HashSet::is_empty) {
            return Ok(matcher);
        }
        let mut files = conn.prepare_cached(
            "SELECT rf.recording_id, f.raw_tags
             FROM file f JOIN recording_file rf ON rf.file_id = f.id
             WHERE f.present = 1 AND f.hidden_reason IS NULL
               AND (f.raw_tags LIKE '%TSRC%' OR f.raw_tags LIKE '%ISRC%')
               AND (?1 IS NULL OR rf.recording_id IN (SELECT value FROM json_each(?1)))
             ORDER BY f.id DESC",
        )?;
        let mut rows = files.query([&among])?;
        while let Some(row) = rows.next()? {
            let (recording, raw): (i64, String) = (row.get(0)?, row.get(1)?);
            for isrc in isrcs_in(&raw) {
                if wanted_isrcs.as_ref().is_some_and(|w| !w.contains(&isrc)) {
                    continue;
                }
                // The lowest track id keeps an ISRC two tracks carry.
                matcher
                    .by_isrc
                    .entry(isrc)
                    .and_modify(|held| *held = (*held).min(recording))
                    .or_insert(recording);
            }
        }
        Ok(matcher)
    }

    /// The candidates that share an artist key with `artists`, in order,
    /// each once.
    fn by_artists(&self, artists: &[String]) -> Vec<usize> {
        let mut at: Vec<usize> = artists
            .iter()
            .filter_map(|key| self.by_artist.get(key))
            .flatten()
            .copied()
            .collect();
        at.sort_unstable();
        at.dedup();
        at
    }

    pub(super) fn owned(&self, recording: i64, by_isrc: bool) -> WishMatch {
        let at = self.by_recording.get(&recording);
        let found = at.map(|&at| &self.candidates[at]);
        let in_library = found.is_some_and(|c| c.in_library);
        let key = match (by_isrc, in_library) {
            (true, true) => keys::OWNED_ISRC_LIBRARY,
            (true, false) => keys::OWNED_ISRC_ALL_MUSIC,
            (false, true) => keys::OWNED_LIBRARY,
            (false, false) => keys::OWNED_ALL_MUSIC,
        };
        WishMatch {
            kind: WishMatchKind::Owned,
            reason: fact(key, found.map(|c| c.title.as_str())),
            recording_id: Some(recording),
            in_library,
        }
    }

    /// The track an owned file's ISRC names, when `wish` has that ISRC.
    pub(super) fn by_isrc_of(&self, wish: &WishTrack) -> Option<i64> {
        let isrc = wish.isrc.as_deref().and_then(normalize_isrc)?;
        let recording = *self.by_isrc.get(&isrc)?;
        self.by_recording
            .contains_key(&recording)
            .then_some(recording)
    }

    /// Every track `wish` is owned as, the very same version (not another
    /// cut, not a rework), each with its reason, the lowest id first. An
    /// ISRC an owned file carries decides alone, as in [`Matcher::find`].
    pub fn owners(&self, wish: &WishTrack) -> Vec<WishMatch> {
        if let Some(recording) = self.by_isrc_of(wish) {
            return vec![self.owned(recording, true)];
        }
        let parsed = versions::parse_title(&wish_name(wish));
        let artists = artist_keys(&wish.artist);
        let wish_keys = title_keys(&parsed);
        let mut found: Vec<i64> = Vec::new();
        for at in self.by_artists(&artists) {
            let candidate = &self.candidates[at];
            let read = candidate.read();
            if !read.keys.iter().any(|k| wish_keys.contains(k)) {
                continue;
            }
            if versions::compare(&parsed, &read.parsed).outcome == Outcome::SameVersion {
                found.push(candidate.recording);
            }
        }
        found.sort_unstable();
        found.dedup();
        found.into_iter().map(|r| self.owned(r, false)).collect()
    }

    /// What `wish` comes to.
    pub fn find(&self, wish: &WishTrack) -> WishMatch {
        if let Some(recording) = self.by_isrc_of(wish) {
            return self.owned(recording, true);
        }
        let parsed = versions::parse_title(&wish_name(wish));
        let artists = artist_keys(&wish.artist);
        let wish_keys = title_keys(&parsed);
        // Same version, other cut, other rework: 3, 2, 1.
        let mut best: Option<(u8, bool, std::cmp::Reverse<i64>, usize)> = None;
        for at in self.by_artists(&artists) {
            let candidate = &self.candidates[at];
            let read = candidate.read();
            if !read.keys.iter().any(|k| wish_keys.contains(k)) {
                continue;
            }
            let rank = match versions::compare(&parsed, &read.parsed).outcome {
                Outcome::SameVersion => 3,
                Outcome::DifferentCut => 2,
                Outcome::DifferentRework => 1,
                Outcome::DifferentBase | Outcome::CantTell => continue,
            };
            let ranked = (
                rank,
                candidate.in_library,
                std::cmp::Reverse(candidate.recording),
                at,
            );
            if best.is_none_or(|b| ranked > b) {
                best = Some(ranked);
            }
        }
        let Some((rank, in_library, _, at)) = best else {
            return missing();
        };
        let candidate = &self.candidates[at];
        let (kind, key) = match (rank, in_library) {
            (3, _) => return self.owned(candidate.recording, false),
            (2, true) => (WishMatchKind::OwnedOtherCut, keys::OTHER_CUT_LIBRARY),
            (2, false) => (WishMatchKind::OwnedOtherCut, keys::OTHER_CUT_ALL_MUSIC),
            _ => (WishMatchKind::Rework, keys::REWORK),
        };
        WishMatch {
            kind,
            reason: fact(key, Some(&candidate.title)),
            recording_id: Some(candidate.recording),
            in_library,
        }
    }
}
