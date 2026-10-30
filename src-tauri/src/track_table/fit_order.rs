//! A crate's table sorted by fit (1cB-16; [`TableSource::CrateByFit`]): the
//! crate's tracks that the search and the filters let through, arranged by
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

/// Every track the query lets through, best fit to `after` first.
fn arranged(
    conn: &Connection,
    query: &TableQuery,
    id: CrateId,
    after: i64,
) -> rusqlite::Result<Vec<(i64, RowFit)>> {
    let quality = match query.filters.contains(&TableFilter::QualityProblem) {
        true => Some(filters::quality_problems(conn)?),
        false => None,
    };
    // The crate in its own order, with the same search and filters.
    let plain = TableQuery {
        source: TableSource::Crate { id },
        sort: None,
        ..query.clone()
    };
    let built = build(&plain, quality.as_deref());
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT tr.recording_id, tr.bpm, tr.key_order {} ORDER BY {}",
        built.from_where, built.order_by
    ))?;
    let candidates = stmt
        .query_map(rusqlite::params_from_iter(built.params.iter()), |r| {
            Ok(Candidate {
                id: r.get(0)?,
                facts: Facts {
                    bpm: r.get(1)?,
                    key: r.get::<_, Option<i64>>(2)?.and_then(Camelot::from_order),
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(arrange(after, target_facts(conn, after)?, &candidates))
}

/// The window of a fit-sorted query: the ids, the count when asked, and each
/// row's fit. `None` for any other source.
#[allow(clippy::type_complexity)]
pub(super) fn window(
    conn: &Connection,
    query: &TableQuery,
    most: u32,
) -> rusqlite::Result<Option<(Vec<i64>, Option<u32>, HashMap<i64, RowFit>)>> {
    let TableSource::CrateByFit { id, after } = &query.source else {
        return Ok(None);
    };
    let all = arranged(conn, query, *id, *after)?;
    let total = query
        .with_total
        .then(|| u32::try_from(all.len()).unwrap_or(u32::MAX));
    let limit = query.limit.min(most) as usize;
    let mut ids = Vec::new();
    let mut fits = HashMap::new();
    for (track, fit) in all.into_iter().skip(query.offset as usize).take(limit) {
        ids.push(track);
        fits.insert(track, fit);
    }
    Ok(Some((ids, total, fits)))
}

/// The ids of all the rows of a fit-sorted query, in order; `None` for any
/// other source.
pub(super) fn ids(conn: &Connection, query: &TableQuery) -> rusqlite::Result<Option<Vec<i64>>> {
    let TableSource::CrateByFit { id, after } = &query.source else {
        return Ok(None);
    };
    Ok(Some(
        arranged(conn, query, *id, *after)?
            .into_iter()
            .map(|(track, _)| track)
            .collect(),
    ))
}
