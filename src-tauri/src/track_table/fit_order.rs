//! A crate's or playlist's table sorted by fit (1cB-16, 4B-FU;
//! [`TableSource::CrateByFit`], [`TableSource::PlaylistByFit`]): the
//! list's tracks that the search and the filters let through, arranged by
//! [`crate::crates::fit::arrange`], and cut into windows.
//!
//! A crate is a few thousand tracks, not the Library, so the whole list is
//! read each time (the track's id, BPM and key from `track_row`, in the
//! crate's own order), arranged in memory, and the window cut from it: the
//! same cost as the table's sorted crate (which also sorts its rows). The
//! BPM and key are the ones the table sorts by, so the order and the
//! columns agree.

use std::collections::HashMap;

use rusqlite::OptionalExtension;

use super::*;
use crate::crates::fit::{arrange, Camelot, Candidate, Facts};

/// The selected track's BPM and key; none for either if the index has no
/// row for it.
fn target_facts(conn: &Connection, after: i64) -> rusqlite::Result<Facts> {
    let found = conn
        .query_row(
            "SELECT bpm, key_order FROM track_row WHERE recording_id = ?1",
            [after],
            |r| Ok((r.get::<_, Option<f64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;
    let (bpm, order) = found.unwrap_or((None, None));
    Ok(Facts {
        bpm,
        key: order.and_then(Camelot::from_order),
    })
}

/// A row of the arranged list: the track and, in a playlist, its entry.
type Arranged = Vec<(i64, Option<i64>)>;

/// Every track the query lets through, best fit to `after` first, with the
/// fit of each track.
fn arranged(
    conn: &Connection,
    query: &TableQuery,
    after: i64,
) -> rusqlite::Result<(Arranged, HashMap<i64, RowFit>)> {
    let quality = match query.filters.contains(&TableFilter::QualityProblem) {
        true => Some(filters::quality_problems(conn)?),
        false => None,
    };
    // The list in its own order, with the same search and filters.
    let source = match &query.source {
        TableSource::CrateByFit { id, .. } => TableSource::Crate { id: *id },
        TableSource::PlaylistByFit { id, .. } => TableSource::Playlist { id: *id },
        other => other.clone(),
    };
    let plain = TableQuery {
        source,
        sort: None,
        ..query.clone()
    };
    let built = build(&plain, quality.as_deref());
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT tr.recording_id, {}, tr.bpm, tr.key_order {} ORDER BY {}",
        built.entry, built.from_where, built.order_by
    ))?;
    let listed = stmt
        .query_map(rusqlite::params_from_iter(built.params.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                Facts {
                    bpm: r.get(2)?,
                    key: r.get::<_, Option<i64>>(3)?.and_then(Camelot::from_order),
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let candidates: Vec<Candidate> = listed
        .iter()
        .map(|&(id, _, facts)| Candidate { id, facts })
        .collect();
    let fits = arrange(after, target_facts(conn, after)?, &candidates);
    // A track in a playlist twice has two entries, taken in the playlist's
    // order; the selected track's entries all come first.
    let mut entries: HashMap<i64, std::collections::VecDeque<Option<i64>>> = HashMap::new();
    for &(id, entry, _) in &listed {
        entries.entry(id).or_default().push_back(entry);
    }
    let mut rows = Vec::with_capacity(listed.len());
    let mut by_track = HashMap::new();
    for (id, fit) in fits {
        if id == after {
            while let Some(entry) = entries.get_mut(&id).and_then(|e| e.pop_front()) {
                rows.push((id, entry));
            }
        } else if let Some(entry) = entries.get_mut(&id).and_then(|e| e.pop_front()) {
            rows.push((id, entry));
        }
        by_track.insert(id, fit);
    }
    Ok((rows, by_track))
}

/// The id and playlist-entry pair of `source`'s fit target, if it has one.
fn target_of(source: &TableSource) -> Option<i64> {
    match source {
        TableSource::CrateByFit { after, .. } | TableSource::PlaylistByFit { after, .. } => {
            Some(*after)
        }
        _ => None,
    }
}

/// The window of a fit-sorted query: the ids, the count when asked, each
/// row's fit, and each row's playlist entry. `None` for any other source.
#[allow(clippy::type_complexity)]
pub(super) fn window(
    conn: &Connection,
    query: &TableQuery,
    most: u32,
) -> rusqlite::Result<
    Option<(
        Vec<i64>,
        Option<u32>,
        HashMap<i64, RowFit>,
        Vec<Option<i64>>,
    )>,
> {
    let Some(after) = target_of(&query.source) else {
        return Ok(None);
    };
    let (all, fits) = arranged(conn, query, after)?;
    let total = query
        .with_total
        .then(|| u32::try_from(all.len()).unwrap_or(u32::MAX));
    let limit = query.limit.min(most) as usize;
    let mut ids = Vec::new();
    let mut entries = Vec::new();
    for (track, entry) in all.into_iter().skip(query.offset as usize).take(limit) {
        ids.push(track);
        entries.push(entry);
    }
    Ok(Some((ids, total, fits, entries)))
}

/// The ids of all the rows of a fit-sorted query, in order; `None` for any
/// other source.
pub(super) fn ids(conn: &Connection, query: &TableQuery) -> rusqlite::Result<Option<Vec<i64>>> {
    let Some(after) = target_of(&query.source) else {
        return Ok(None);
    };
    Ok(Some(
        arranged(conn, query, after)?
            .0
            .into_iter()
            .map(|(track, _)| track)
            .collect(),
    ))
}
