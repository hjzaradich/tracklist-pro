//! The queue: how many items wait, and a page of them. See the module docs
//! of [`super`] for what each kind is.
//!
//! Every kind has one SQL text for its WHERE ([`version_doubt_from`] and the
//! like), used by the count, the page and the check that one item is still
//! queued ([`super::compare`]), so the three cannot disagree.

use std::collections::HashMap;

use rusqlite::{params, Connection, Params};

use super::actions::{content_key, DISMISSED_BETTER, KEEP_ANYWAY, WANT_BETTER};
use super::held::{self, Component};
use super::{
    ReviewCounts, ReviewGroup, ReviewItem, ReviewItemId, ReviewKind, ReviewPage, ReviewTrackName,
    FLAGGED, MAX_NAMES, PENDING_PAIRS, PROBABLE_RELINK,
};
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::quality::store::INPUTS;
use crate::quality::RULES_VERSION;
use crate::send_values::{shown, shown_recordings};

// ---- the WHERE of each kind -------------------------------------------

/// The `FROM … WHERE` of the version doubts: one row per pair of files; the
/// item is the pair of tracks. `{extra}` is added to the WHERE.
pub(super) fn version_doubt_from(extra: &str) -> String {
    format!(
        "FROM grouping_pair
         JOIN recording_file ra ON ra.file_id = grouping_pair.file_a
         JOIN recording_file rb ON rb.file_id = grouping_pair.file_b
         WHERE {PENDING_PAIRS} AND state = 'open' AND ra.recording_id <> rb.recording_id
           AND (ra.recording_id IN (SELECT recording_id FROM library_track)
                OR rb.recording_id IN (SELECT recording_id FROM library_track))
           {extra}"
    )
}

/// The track pair of a version-doubt row, lower id first.
pub(super) const DOUBT_PAIR: &str =
    "min(ra.recording_id, rb.recording_id), max(ra.recording_id, rb.recording_id)";

/// The `FROM … WHERE` of the relink items.
pub(super) fn relink_from(extra: &str) -> String {
    format!("FROM rekordbox_track WHERE {PROBABLE_RELINK} {extra}")
}

/// The `FROM … WHERE` of the Library tracks with a better file, by step:
/// `bf.step = 'onDisk'` (the linked file is gone) or not. `lf` is the
/// linked file and `bfile` the better one.
///
/// A "Better file available" item the user dismissed isn't one: a
/// `dismissed_better` decision for the audio the linked file holds and the
/// audio that better file holds ([`content_key`]). Another better file has
/// another key, so it is an item again. A "Linked file gone, copy found"
/// item can't be dismissed.
pub(super) fn better_from(on_disk: bool, extra: &str) -> String {
    let step = if on_disk {
        "bf.step = 'onDisk'".to_owned()
    } else {
        format!(
            "bf.step <> 'onDisk'
             AND NOT EXISTS (SELECT 1 FROM review_decision d
                             WHERE d.kind = '{DISMISSED_BETTER}'
                               AND d.linked_key = {} AND d.other_key = {})",
            content_key("lf"),
            content_key("bfile")
        )
    };
    format!(
        "FROM better_file bf
         JOIN library_track lt ON lt.recording_id = bf.recording_id
         JOIN file bfile ON bfile.id = bf.file_id
         LEFT JOIN file lf ON lf.id = lt.linked_file_id
         WHERE {step} {extra}"
    )
}

/// The `FROM … WHERE` of the quality problems: a Library track whose linked
/// file is on disk and whose stored verdict, as long as it is current,
/// flags it, with no better copy; and that the user hasn't answered ("Keep
/// anyway" or "Find a better file": a decision for the audio the linked
/// file holds).
pub(super) fn quality_from(extra: &str) -> String {
    format!(
        "FROM file_verdict v
         JOIN file f ON f.id = v.file_id
         LEFT JOIN file_quality q ON q.file_id = f.id
         JOIN library_track lt ON lt.linked_file_id = f.id
         WHERE {FLAGGED} AND v.rules_version = {RULES_VERSION} AND v.inputs IS {INPUTS}
           AND f.present = 1
           AND NOT EXISTS (SELECT 1 FROM better_file bf WHERE bf.recording_id = lt.recording_id)
           AND NOT EXISTS (SELECT 1 FROM review_decision d
                           WHERE d.kind IN ('{KEEP_ANYWAY}', '{WANT_BETTER}')
                             AND d.linked_key = {})
           {extra}",
        content_key("f")
    )
}

/// The tracks of the "Quality problems" group, as a `SELECT` of their ids:
/// the quality problems ([`quality_from`]) and the "Better file available"
/// items whose linked file is still there ([`better_from`]). The one SQL
/// text the track table's "Quality problem" chip reads, so the chip and the
/// group can't mean different things; a test holds the chip to the group's
/// items.
pub(crate) fn quality_problem_recordings() -> String {
    format!(
        "SELECT lt.recording_id {} UNION SELECT lt.recording_id {}",
        quality_from(""),
        better_from(false, "")
    )
}

// ---- counts ---------------------------------------------------------------

fn count(conn: &Connection, sql: &str, params: impl Params) -> rusqlite::Result<u32> {
    conn.prepare_cached(sql)?.query_row(params, |r| r.get(0))
}

/// How many items of each kind wait.
fn kind_counts(
    conn: &Connection,
    wanted: &[ReviewKind],
    sets: &[Component],
) -> rusqlite::Result<HashMap<ReviewKind, u32>> {
    let sets_of = |same: bool| sets.iter().filter(|c| c.same_recording == same).count() as u32;
    let mut counts = HashMap::new();
    for &kind in wanted {
        let n = match kind {
            ReviewKind::VersionDoubt => count(
                conn,
                &format!(
                    "SELECT count(*) FROM (SELECT 1 {} GROUP BY {DOUBT_PAIR})",
                    version_doubt_from("")
                ),
                [],
            )?,
            ReviewKind::HeldSet => sets_of(false),
            ReviewKind::SameRecording => sets_of(true),
            ReviewKind::Relink => count(conn, &format!("SELECT count(*) {}", relink_from("")), [])?,
            ReviewKind::ClearBestFile => count(
                conn,
                &format!("SELECT count(*) {}", better_from(true, "")),
                [],
            )?,
            ReviewKind::QualityProblem => {
                count(conn, &format!("SELECT count(*) {}", quality_from("")), [])?
            }
            ReviewKind::BetterFile => count(
                conn,
                &format!("SELECT count(*) {}", better_from(false, "")),
                [],
            )?,
        };
        counts.insert(kind, n);
    }
    Ok(counts)
}

/// The held sets, if any of `kinds` needs them.
fn sets_for(conn: &Connection, kinds: &[ReviewKind]) -> rusqlite::Result<Vec<Component>> {
    if kinds
        .iter()
        .any(|k| matches!(k, ReviewKind::HeldSet | ReviewKind::SameRecording))
    {
        held::components(conn)
    } else {
        Ok(Vec::new())
    }
}

/// How many items wait, by group. What the Overview reuses ("how many need
/// a decision" is [`ReviewCounts::total`]).
pub fn counts(conn: &Connection) -> rusqlite::Result<ReviewCounts> {
    // One read transaction: the kinds are counted as of the same moment.
    let conn = conn.unchecked_transaction()?;
    let conn = &*conn;
    let sets = sets_for(conn, &ReviewKind::ALL)?;
    let by_kind = kind_counts(conn, &ReviewKind::ALL, &sets)?;
    let of = |group: ReviewGroup| -> u32 { group.kinds().map(|k| by_kind[&k]).sum() };
    let (needs_closer_look, clear_best_file, quality_problems) = (
        of(ReviewGroup::NeedsCloserLook),
        of(ReviewGroup::ClearBestFile),
        of(ReviewGroup::QualityProblems),
    );
    Ok(ReviewCounts {
        needs_closer_look,
        clear_best_file,
        quality_problems,
        total: needs_closer_look + clear_best_file + quality_problems,
    })
}

// ---- pages ----------------------------------------------------------------

/// An item before its tracks are named.
struct Found {
    id: ReviewItemId,
    /// The tracks, the first of them being the item's own.
    tracks: Vec<i64>,
    /// A relink item's rekordbox entry: its name and artist.
    entry: Option<(Option<String>, Option<String>)>,
}

/// The items of one kind from `offset` on, at most `limit`.
fn found_of_kind(
    conn: &Connection,
    kind: ReviewKind,
    sets: &[Component],
    offset: u32,
    limit: u32,
) -> rusqlite::Result<Vec<Found>> {
    let paged = "ORDER BY 1 LIMIT ?1 OFFSET ?2";
    let by_track = |id: i64| Found {
        id: ReviewItemId {
            kind,
            a: id,
            b: None,
        },
        tracks: vec![id],
        entry: None,
    };
    match kind {
        ReviewKind::VersionDoubt => {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {DOUBT_PAIR} {} GROUP BY 1, 2 ORDER BY 1, 2 LIMIT ?1 OFFSET ?2",
                version_doubt_from("")
            ))?;
            let rows = stmt.query_map(params![limit, offset], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?;
            rows.map(|row| {
                let (a, b) = row?;
                Ok(Found {
                    id: ReviewItemId {
                        kind,
                        a,
                        b: Some(b),
                    },
                    tracks: vec![a, b],
                    entry: None,
                })
            })
            .collect()
        }
        ReviewKind::HeldSet | ReviewKind::SameRecording => Ok(sets
            .iter()
            .filter(|c| c.same_recording == (kind == ReviewKind::SameRecording))
            .skip(offset as usize)
            .take(limit as usize)
            .map(|c| Found {
                id: ReviewItemId {
                    kind,
                    a: c.anchor,
                    b: None,
                },
                tracks: c.tracks.clone(),
                entry: None,
            })
            .collect()),
        ReviewKind::Relink => {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT id, json_extract(attributes, '$.Name'), json_extract(attributes, '$.Artist')
                 {} {paged}",
                relink_from("")
            ))?;
            let rows = stmt.query_map(params![limit, offset], |r| {
                Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?))
            })?;
            rows.map(|row| {
                let (id, title, artist) = row?;
                Ok(Found {
                    id: ReviewItemId {
                        kind,
                        a: id,
                        b: None,
                    },
                    tracks: Vec::new(),
                    entry: Some((title, artist)),
                })
            })
            .collect()
        }
        ReviewKind::ClearBestFile | ReviewKind::BetterFile => {
            let on_disk = kind == ReviewKind::ClearBestFile;
            by_recording(
                conn,
                &format!(
                    "SELECT lt.recording_id {} ORDER BY 1 LIMIT ?1 OFFSET ?2",
                    better_from(on_disk, "")
                ),
                offset,
                limit,
                by_track,
            )
        }
        ReviewKind::QualityProblem => by_recording(
            conn,
            &format!(
                "SELECT lt.recording_id {} ORDER BY 1 LIMIT ?1 OFFSET ?2",
                quality_from("")
            ),
            offset,
            limit,
            by_track,
        ),
    }
}

fn by_recording(
    conn: &Connection,
    sql: &str,
    offset: u32,
    limit: u32,
    make: impl Fn(i64) -> Found,
) -> rusqlite::Result<Vec<Found>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![limit, offset], |r| r.get::<_, i64>(0))?;
    rows.map(|id| Ok(make(id?))).collect()
}

/// A page of `group`: its items from `offset` on, at most `limit`, in the
/// order of [`ReviewKind::ALL`] and then oldest first, with their tracks
/// named.
pub fn items(
    conn: &Connection,
    volumes: &impl Volumes,
    group: ReviewGroup,
    offset: u32,
    limit: u32,
) -> rusqlite::Result<ReviewPage> {
    // One read transaction for the counts and the page: the window over the
    // kinds is computed from the counts, so a write between the two reads
    // would shift later kinds by one.
    let conn = conn.unchecked_transaction()?;
    let conn = &*conn;
    let kinds: Vec<ReviewKind> = group.kinds().collect();
    let sets = sets_for(conn, &kinds)?;
    let by_kind = kind_counts(conn, &kinds, &sets)?;
    let total: u32 = kinds.iter().map(|k| by_kind[k]).sum();

    // The page is a window over the kinds laid end to end.
    let mut found = Vec::new();
    let (mut start, end) = (offset, offset.saturating_add(limit));
    let mut before = 0u32;
    for kind in kinds {
        let size = by_kind[&kind];
        let (lo, hi) = (start.max(before), end.min(before + size));
        if lo < hi {
            found.extend(found_of_kind(conn, kind, &sets, lo - before, hi - lo)?);
            start = hi;
        }
        before += size;
    }

    let names = names(
        conn,
        volumes,
        found.iter().flat_map(|f| f.tracks.iter().copied()),
    )?;
    let items = found
        .into_iter()
        .map(|f| {
            let more_tracks = f.tracks.len().saturating_sub(MAX_NAMES) as u32;
            let mut tracks: Vec<ReviewTrackName> = match f.entry {
                Some((title, artist)) => vec![ReviewTrackName {
                    recording_id: None,
                    title,
                    artist,
                }],
                None => Vec::new(),
            };
            tracks.extend(
                f.tracks
                    .iter()
                    .take(MAX_NAMES)
                    .map(|id| name_of(&names, *id)),
            );
            ReviewItem {
                id: f.id,
                kind: f.id.kind,
                tracks,
                more_tracks,
            }
        })
        .collect();
    Ok(ReviewPage {
        group,
        total,
        items,
    })
}

/// The title and artist shown for each track, by track id.
pub(super) type Names = HashMap<i64, (Option<String>, Option<String>)>;

/// The title and artist shown for each track: a Library track's as the
/// Library list shows them, any other track's from its own files.
pub(super) fn names(
    conn: &Connection,
    volumes: &impl Volumes,
    recordings: impl Iterator<Item = i64>,
) -> rusqlite::Result<Names> {
    let wanted: Vec<i64> = recordings.collect();
    let json = serde_json::to_string(&wanted).unwrap_or_else(|_| "[]".into());
    let library: HashMap<i64, i64> = {
        let mut stmt = conn.prepare_cached(
            "SELECT recording_id, id FROM library_track
             WHERE recording_id IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = stmt.query_map([&json], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let library_ids: Vec<LibraryTrackId> = library.values().map(|id| LibraryTrackId(*id)).collect();
    let others: Vec<i64> = wanted
        .iter()
        .copied()
        .filter(|r| !library.contains_key(r))
        .collect();
    let mut library_names = shown(conn, volumes, &library_ids)?;
    let mut other_names = shown_recordings(conn, &others)?;
    let mut names = HashMap::new();
    for &recording in &wanted {
        let found = match library.get(&recording) {
            Some(id) => library_names.remove(&LibraryTrackId(*id)),
            None => other_names.remove(&recording),
        };
        if let Some(found) = found {
            names.insert(recording, found.into_options());
        }
    }
    Ok(names)
}

pub(super) fn name_of(names: &Names, recording: i64) -> ReviewTrackName {
    let (title, artist) = names.get(&recording).cloned().unwrap_or((None, None));
    ReviewTrackName {
        recording_id: Some(recording),
        title,
        artist,
    }
}
