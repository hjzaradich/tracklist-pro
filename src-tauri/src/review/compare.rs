//! The comparison of one queue item (1bD-2; ROADMAP 1.12): one row per file
//! with the same columns, the suggested file first, and the reason for the
//! suggestion as a code plus parameters ([`super::reasons`]).
//!
//! The rows are made from the facts the best-file ranking itself reads
//! ([`crate::file_roles::facts`]), so what a row shows is what the ranking
//! saw. The suggestion is the stored "better file" where the item is about
//! one (a Library track with a copy ahead of its linked file); otherwise the
//! live ranking's best file. A quality problem has no suggestion: there is
//! no better copy to name.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use super::held;
use super::queue::{
    better_from, name_of, names, quality_from, relink_from, version_doubt_from, DOUBT_PAIR,
};
use super::reasons::{from_grouping, from_holding, from_rank, from_relink};
use super::{
    RekordboxUse, ReviewComparison, ReviewFileRow, ReviewItemId, ReviewKind, ReviewTrackName,
    RowSource, MAX_FILES, PENDING_PAIRS,
};
use crate::best_file::{
    rank, tags_filled, why_ahead, FileFacts, RekordboxLink, COMPLETENESS_FIELDS,
};
use crate::file_roles::facts::{self, Loaded};
use crate::library::{file_from, LinkedFile, StoredFile, FILE_COLUMNS, FILE_JOINS};
use crate::paths::Volumes;
use crate::suggest::{Reason, Suggestion};

/// What the ranking doesn't read about a file.
struct Detail {
    stored: Option<StoredFile>,
    rel_path: String,
    duration_ms: Option<u32>,
    bitrate_kbps: Option<u32>,
    sample_rate_hz: Option<u32>,
    added_at: Option<String>,
    format: Option<String>,
}

fn details(conn: &Connection, files: &[i64]) -> rusqlite::Result<HashMap<i64, Detail>> {
    let json = serde_json::to_string(files).unwrap_or_else(|_| "[]".into());
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {FILE_COLUMNS}, f.duration_ms, f.bitrate, f.sample_rate, f.first_seen_at,
                f.sniffed_format
         FROM file f {FILE_JOINS}
         WHERE f.id IN (SELECT value FROM json_each(?1))"
    ))?;
    let rows = stmt.query_map([json], |r| {
        let file_id: i64 = r.get(0)?;
        let number = |at: usize| -> rusqlite::Result<Option<u32>> {
            Ok(r.get::<_, Option<i64>>(at)?
                .and_then(|n| u32::try_from(n).ok()))
        };
        Ok((
            file_id,
            Detail {
                stored: file_from(r, 0)?,
                rel_path: r.get(3)?,
                duration_ms: number(6)?,
                bitrate_kbps: number(7)?,
                sample_rate_hz: number(8)?,
                added_at: r.get(9)?,
                format: r.get(10)?,
            },
        ))
    })?;
    rows.collect()
}

/// What an item turned out to be, once it is known to be queued.
struct Resolved {
    /// The tracks, in the order the item lists them.
    tracks: Vec<i64>,
    /// The grouping's stored reasons for the item's pair, if it has one.
    reasons: Option<String>,
    /// The file the item is about, if it is about one.
    about: Option<i64>,
    /// A relink item's entry.
    entry: Option<Entry>,
}

/// rekordbox's entry in a relink item.
struct Entry {
    file: i64,
    method: String,
    /// The user confirmed this entry on this file before (and it is asked
    /// again: the file holds other audio now).
    confirmed_before: bool,
    title: Option<String>,
    artist: Option<String>,
    path: Option<String>,
    duration_ms: Option<u32>,
    bitrate_kbps: Option<u32>,
    sample_rate_hz: Option<u32>,
}

/// The tracks of the item `id` while it is in the queue, as the comparison
/// shows them: the two of a version doubt, every track of a held set.
/// `None` once it isn't queued.
pub(super) fn item_tracks(
    conn: &Connection,
    id: ReviewItemId,
) -> rusqlite::Result<Option<Vec<i64>>> {
    Ok(resolve(conn, id)?.map(|item| item.tracks))
}

fn resolve(conn: &Connection, id: ReviewItemId) -> rusqlite::Result<Option<Resolved>> {
    let plain = |tracks: Vec<i64>| Resolved {
        tracks,
        reasons: None,
        about: None,
        entry: None,
    };
    match id.kind {
        ReviewKind::VersionDoubt => {
            let Some(b) = id.b else { return Ok(None) };
            let reasons: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT grouping_pair.reasons {}
                         ORDER BY grouping_pair.file_a, grouping_pair.file_b LIMIT 1",
                        version_doubt_from(&format!("AND ({DOUBT_PAIR}) = (?1, ?2)"))
                    ),
                    [id.a, b],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(reasons.map(|reasons| Resolved {
                reasons: Some(reasons),
                ..plain(vec![id.a, b])
            }))
        }
        ReviewKind::HeldSet | ReviewKind::SameRecording => {
            let same = id.kind == ReviewKind::SameRecording;
            let Some(set) = held::components(conn)?
                .into_iter()
                .find(|c| c.anchor == id.a && c.same_recording == same)
            else {
                return Ok(None);
            };
            let json = serde_json::to_string(&set.tracks).unwrap_or_else(|_| "[]".into());
            let reasons: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT reasons FROM grouping_pair
                         JOIN recording_file ra ON ra.file_id = grouping_pair.file_a
                         JOIN recording_file rb ON rb.file_id = grouping_pair.file_b
                         WHERE {PENDING_PAIRS} AND state <> 'open'
                           AND ra.recording_id IN (SELECT value FROM json_each(?1))
                           AND rb.recording_id IN (SELECT value FROM json_each(?1))
                         ORDER BY (state = 'both_in_library') DESC,
                                  grouping_pair.file_a, grouping_pair.file_b LIMIT 1"
                    ),
                    [json],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(Some(Resolved {
                reasons,
                ..plain(set.tracks)
            }))
        }
        ReviewKind::Relink => relink(conn, id.a),
        ReviewKind::ClearBestFile | ReviewKind::BetterFile => {
            let file: Option<i64> = conn
                .query_row(
                    &format!(
                        "SELECT bf.file_id {}",
                        better_from(
                            id.kind == ReviewKind::ClearBestFile,
                            "AND lt.recording_id = ?1"
                        )
                    ),
                    [id.a],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(file.map(|file| Resolved {
                about: Some(file),
                ..plain(vec![id.a])
            }))
        }
        ReviewKind::QualityProblem => {
            let queued: bool = conn.query_row(
                &format!(
                    "SELECT EXISTS (SELECT 1 {})",
                    quality_from("AND lt.recording_id = ?1")
                ),
                [id.a],
                |r| r.get(0),
            )?;
            Ok(queued.then(|| plain(vec![id.a])))
        }
    }
}

fn relink(conn: &Connection, row: i64) -> rusqlite::Result<Option<Resolved>> {
    let found = conn
        .query_row(
            &format!(
                "SELECT file_id, relink_method, location_key,
                        json_extract(attributes, '$.Name'), json_extract(attributes, '$.Artist'),
                        CASE WHEN json_extract(attributes, '$.TotalTime') GLOB '[0-9]*'
                             THEN CAST(json_extract(attributes, '$.TotalTime') AS INTEGER) END,
                        CAST(json_extract(attributes, '$.BitRate') AS INTEGER),
                        CAST(json_extract(attributes, '$.SampleRate') AS INTEGER),
                        (SELECT recording_id FROM recording_file WHERE file_id = rekordbox_track.file_id),
                        EXISTS (SELECT 1 FROM relink r
                                WHERE r.location_key = rekordbox_track.location_key
                                  AND r.verdict = 'same'
                                  AND (r.file_id = rekordbox_track.file_id
                                       OR r.file_key = (SELECT coalesce(nullif(f.audio_hash, x''), f.blake3)
                                                        FROM file f WHERE f.id = rekordbox_track.file_id)))
                 {}",
                relink_from("AND id = ?1")
            ),
            [row],
            |r| {
                let number = |at: usize| -> rusqlite::Result<Option<u32>> {
                    Ok(r.get::<_, Option<i64>>(at)?.and_then(|n| u32::try_from(n).ok()))
                };
                let seconds = number(5)?;
                Ok((
                    Entry {
                        file: r.get(0)?,
                        method: r.get(1)?,
                        confirmed_before: r.get(9)?,
                        path: r
                            .get::<_, Option<String>>(2)?
                            .map(|key| key.replace('/', "\\")),
                        title: r.get(3)?,
                        artist: r.get(4)?,
                        duration_ms: seconds.map(|s| s.saturating_mul(1000)),
                        bitrate_kbps: number(6)?,
                        sample_rate_hz: number(7)?,
                    },
                    r.get::<_, Option<i64>>(8)?,
                ))
            },
        )
        .optional()?;
    Ok(found.map(|(entry, recording)| Resolved {
        tracks: recording.into_iter().collect(),
        reasons: None,
        about: Some(entry.file),
        entry: Some(entry),
    }))
}

fn rekordbox_use(link: RekordboxLink) -> RekordboxUse {
    match link {
        RekordboxLink::Unknown => RekordboxUse::Unknown,
        RekordboxLink::Probable => RekordboxUse::Probable,
        RekordboxLink::Trusted => RekordboxUse::Trusted,
    }
}

/// The file's last path component.
fn last_component(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

/// The name each file is shown by (as a comparison row names it), by file id.
pub(super) fn file_names(
    conn: &Connection,
    volumes: &impl Volumes,
    files: &[i64],
) -> rusqlite::Result<HashMap<i64, String>> {
    Ok(details(conn, files)?
        .into_iter()
        .map(|(file, detail)| {
            let name = detail
                .stored
                .as_ref()
                .map(|stored| stored.shown(volumes).name)
                .unwrap_or_else(|| last_component(&detail.rel_path));
            (file, name)
        })
        .collect())
}

fn row_of(
    recording: i64,
    loaded: &Loaded,
    detail: Option<&Detail>,
    linked: &HashSet<i64>,
    volumes: &impl Volumes,
) -> ReviewFileRow {
    let facts = &loaded.facts;
    let shown: Option<LinkedFile> = detail
        .and_then(|d| d.stored.as_ref())
        .map(|stored| stored.shown(volumes));
    let rel_path = detail.map_or("", |d| d.rel_path.as_str());
    ReviewFileRow {
        file_id: Some(facts.file_id),
        source: RowSource::File,
        recording_id: Some(recording).filter(|track| *track != 0),
        name: shown
            .as_ref()
            .map_or_else(|| last_component(rel_path), |s| s.name.clone()),
        path: shown.as_ref().map(|s| s.path.clone()),
        on_disk: facts.on_disk,
        drive_connected: shown.as_ref().is_some_and(|s| s.drive_connected),
        linked: linked.contains(&facts.file_id),
        format: detail.and_then(|d| d.format.clone()),
        verdict: facts.quality.verdict,
        cutoff_hz: facts.quality.cutoff_hz,
        duration_ms: detail.and_then(|d| d.duration_ms),
        tags_filled: tags_filled(&facts.tags),
        tags_total: COMPLETENESS_FIELDS.len() as u32,
        has_art: facts.art.is_some(),
        rekordbox: rekordbox_use(facts.rekordbox),
        bitrate_kbps: detail.and_then(|d| d.bitrate_kbps),
        sample_rate_hz: detail.and_then(|d| d.sample_rate_hz),
        added_at: detail.and_then(|d| d.added_at.clone()),
    }
}

fn entry_row(entry: &Entry) -> ReviewFileRow {
    ReviewFileRow {
        file_id: None,
        source: RowSource::RekordboxEntry,
        recording_id: None,
        name: entry
            .path
            .as_deref()
            .map_or_else(String::new, last_component),
        path: entry.path.clone(),
        on_disk: false,
        drive_connected: false,
        linked: false,
        format: None,
        verdict: None,
        cutoff_hz: None,
        duration_ms: entry.duration_ms,
        tags_filled: 0,
        tags_total: COMPLETENESS_FIELDS.len() as u32,
        has_art: false,
        rekordbox: RekordboxUse::Trusted,
        bitrate_kbps: entry.bitrate_kbps,
        sample_rate_hz: entry.sample_rate_hz,
        added_at: None,
    }
}

/// The comparison for `id`, or `None` if the item is no longer in the queue.
pub fn comparison(
    conn: &Connection,
    volumes: &impl Volumes,
    id: ReviewItemId,
) -> rusqlite::Result<Option<ReviewComparison>> {
    // One read transaction: the item, its files and their names are of the
    // same moment.
    let conn = conn.unchecked_transaction()?;
    let conn = &*conn;
    let Some(item) = resolve(conn, id)? else {
        return Ok(None);
    };

    // The files, by track.
    let by_track = facts::load(conn, &item.tracks)?;
    let mut files: Vec<(i64, &Loaded)> = Vec::new();
    for (&track, loaded) in &by_track {
        for file in loaded {
            if item.entry.is_none() || Some(file.facts.file_id) == item.about {
                files.push((track, file));
            }
        }
    }
    // A relink's matched file that is in no track yet (scanned, not grouped)
    // has a row of its own all the same.
    let unplaced = match &item.entry {
        Some(entry) if !files.iter().any(|(_, l)| l.facts.file_id == entry.file) => {
            facts::load_unplaced(conn, entry.file)?
        }
        _ => None,
    };
    if let Some(loaded) = &unplaced {
        files.push((0, loaded));
    }
    let ids: Vec<i64> = files.iter().map(|(_, l)| l.facts.file_id).collect();
    let details = details(conn, &ids)?;
    let linked: HashSet<i64> = {
        let json = serde_json::to_string(&item.tracks).unwrap_or_else(|_| "[]".into());
        let mut stmt = conn.prepare_cached(
            "SELECT linked_file_id FROM library_track
             WHERE linked_file_id IS NOT NULL
               AND recording_id IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = stmt.query_map([json], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    // The ranking, over every file of the item.
    let all: Vec<FileFacts> = files.iter().map(|(_, l)| l.facts.clone()).collect();
    let ranking = rank(&all);

    // The suggestion: which file, and why.
    let facts_of = |file: i64| all.iter().find(|f| f.file_id == file);
    let ranked_reason = |file: i64| {
        ranking
            .order
            .iter()
            .find(|r| r.file_id == file)
            .map(|r| from_rank(&r.reason))
    };
    // A "Same track twice" item: its Library tracks, and which to keep.
    // Tracks linked as versions are never merged: no track to keep is
    // suggested, and the item offers "Keep both" only.
    let same_recording = match id.kind {
        ReviewKind::SameRecording => {
            let tracks = super::same::offer_tracks(conn, volumes, id.a)?;
            let recordings: Vec<i64> = tracks.iter().map(|t| t.recording_id).collect();
            let linked = super::same::linked_versions(conn, &recordings)?;
            let keep = match linked {
                true => None,
                false => keep_suggestion(&tracks, &ranking, &all, &files),
            };
            Some((tracks, keep, linked))
        }
        _ => None,
    };
    let suggestion: Option<(i64, Reason)> = match id.kind {
        ReviewKind::QualityProblem => None,
        // The linked file of the track to keep, and why that track.
        ReviewKind::SameRecording => same_recording
            .as_ref()
            .and_then(|(_, keep, _)| keep.clone()),
        // A match the user confirmed before is here again for that reason,
        // however it was found the first time.
        ReviewKind::Relink => item.entry.as_ref().map(|e| {
            let found = if e.confirmed_before {
                "user"
            } else {
                e.method.as_str()
            };
            (e.file, from_relink(found))
        }),
        ReviewKind::ClearBestFile | ReviewKind::BetterFile => item.about.and_then(|better| {
            // Why it is ahead of the linked file; else the ranking's reason.
            let linked_file = linked.iter().copied().find(|f| *f != better);
            let ahead = linked_file
                .and_then(facts_of)
                .zip(facts_of(better))
                .and_then(|(linked, better)| why_ahead(better, linked))
                .map(|reason| from_rank(&reason));
            ahead.or_else(|| ranked_reason(better)).map(|r| (better, r))
        }),
        _ => ranking
            .best()
            .and_then(|best| ranked_reason(best).map(|r| (best, r))),
    };
    let suggestion = suggestion.and_then(|(file, reason)| Suggestion::new(file, vec![reason]).ok());

    // Rows: the suggested file first, then the ranking's order.
    let first = suggestion.as_ref().map(|s| *s.what());
    let mut order: Vec<i64> = first.into_iter().collect();
    order.extend(
        ranking
            .order
            .iter()
            .map(|r| r.file_id)
            .filter(|f| Some(*f) != first),
    );
    let mut rows: Vec<ReviewFileRow> = order
        .iter()
        .filter_map(|file| {
            let (track, loaded) = files.iter().find(|(_, l)| l.facts.file_id == *file)?;
            Some(row_of(*track, loaded, details.get(file), &linked, volumes))
        })
        .collect();
    if let Some(entry) = &item.entry {
        rows.push(entry_row(entry));
    }
    let more_files = rows.len().saturating_sub(MAX_FILES) as u32;
    rows.truncate(MAX_FILES);

    // The tracks' names.
    let names = names(conn, volumes, item.tracks.iter().copied())?;
    let mut tracks: Vec<ReviewTrackName> = Vec::new();
    if let Some(entry) = &item.entry {
        tracks.push(ReviewTrackName {
            recording_id: None,
            title: entry.title.clone(),
            artist: entry.artist.clone(),
        });
    }
    tracks.extend(item.tracks.iter().map(|t| name_of(&names, *t)));

    // What switching to the copy means in rekordbox, for the send's own
    // answer as the database is now.
    let switch = match (id.kind, item.about) {
        (ReviewKind::ClearBestFile, Some(copy)) => {
            super::actions::offer(conn, volumes, id.a, copy)?
        }
        _ => None,
    };

    // The switch to each copy on disk, one plan for all of them.
    let copies = if id.kind == ReviewKind::ClearBestFile {
        let wanted: Vec<(i64, i64)> = rows
            .iter()
            .filter(|row| row.source == RowSource::File && row.on_disk && !row.linked)
            .filter_map(|row| row.file_id.map(|file| (id.a, file)))
            .collect();
        super::actions::plan(conn, volumes, &wanted)?
            .iter()
            .map(|p| super::actions::SwitchOffer {
                file_id: p.to,
                effect: p.outcome.ok().map(|e| e.effect),
                refusal: p.outcome.err(),
            })
            .collect()
    } else {
        Vec::new()
    };

    // Why the item is here. A held set says what holds it; with nothing
    // stored to name, it shows the reasons of one of its own duplicate pairs.
    let holding = match id.kind {
        ReviewKind::HeldSet => held::holding(conn, &item.tracks)?,
        _ => None,
    };
    let doubt = match &holding {
        Some(holding) => {
            let names = file_names(conn, volumes, &holding.named_files())?;
            let name_of = |file: i64| names.get(&file).cloned().unwrap_or_default();
            vec![from_holding(holding, name_of)]
        }
        None => item
            .reasons
            .as_deref()
            .map(from_grouping)
            .unwrap_or_default(),
    };

    Ok(Some(ReviewComparison {
        id,
        kind: id.kind,
        group: id.kind.group(),
        tracks,
        doubt,
        suggestion,
        rows,
        more_files,
        switch,
        copies,
        relink: item.entry.as_ref().map(|e| super::actions::RelinkOffer {
            file_id: e.file,
            confirmed_before: e.confirmed_before,
        }),
        same_recording: same_recording.map(|(tracks, keep, linked_versions)| {
            let keep = keep.and_then(|(file, _)| {
                tracks
                    .iter()
                    .find(|t| t.linked_file_id == Some(file))
                    .map(|t| t.recording_id)
            });
            super::same::SameRecordingOffer {
                tracks,
                keep,
                linked_versions,
            }
        }),
    }))
}

/// The track a merge should keep ([`super::same::suggest_keep`]), as the
/// file to mark (its linked file) and the reason, against the runner-up.
fn keep_suggestion(
    tracks: &[super::same::SameRecordingTrack],
    ranking: &crate::best_file::Ranking,
    all: &[FileFacts],
    files: &[(i64, &Loaded)],
) -> Option<(i64, Reason)> {
    use super::same::{suggest_keep, KeepCandidate, KeepWhy};
    // Each track's best-ranked file, and where it stands.
    let best_of = |recording: i64| -> Option<(usize, i64)> {
        ranking.order.iter().enumerate().find_map(|(at, r)| {
            files
                .iter()
                .any(|(track, l)| *track == recording && l.facts.file_id == r.file_id)
                .then_some((at, r.file_id))
        })
    };
    let candidates: Vec<KeepCandidate> = tracks
        .iter()
        .map(|t| KeepCandidate {
            recording: t.recording_id,
            cues: t.rekordbox.map_or(0, |e| e.cues),
            plays: t.rekordbox.map_or(0, |e| e.plays),
            rank: best_of(t.recording_id).map_or(usize::MAX, |(at, _)| at),
        })
        .collect();
    let (keep, why) = suggest_keep(&candidates)?;
    let file = tracks
        .iter()
        .find(|t| t.recording_id == keep)?
        .linked_file_id?;
    let facts_of = |file: i64| all.iter().find(|f| f.file_id == file);
    let reason = match why {
        KeepWhy::MoreCues { cues, other } => keep_reason("review:keep.moreCues", cues, other),
        KeepWhy::MorePlays { plays, other } => keep_reason("review:keep.morePlays", plays, other),
        KeepWhy::BetterFile { other } => best_of(keep)
            .zip(best_of(other))
            .and_then(|((_, a), (_, b))| facts_of(a).zip(facts_of(b)))
            .and_then(|(a, b)| why_ahead(a, b))
            .map(|r| from_rank(&r))
            .unwrap_or_else(|| fact("review:keep.tied")),
        KeepWhy::Tied => fact("review:keep.tied"),
    };
    Some((file, reason))
}

fn fact(key: &str) -> Reason {
    Reason::fact(key).unwrap_or_else(|e| panic!("bad reason key {key}: {e}"))
}

fn keep_reason(key: &str, count: u32, other: u32) -> Reason {
    fact(key)
        .with_number("count", f64::from(count))
        .and_then(|r| r.with_number("other", f64::from(other)))
        .unwrap_or_else(|e| panic!("bad reason parameter: {e}"))
}
