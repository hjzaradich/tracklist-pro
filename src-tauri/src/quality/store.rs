//! The stored verdict per file (ROADMAP 1.6; 1bC-5): reading what the
//! database holds into [`Facts`], keeping [`assess`]'s answer in
//! `file_verdict` (migration 0021), and reading it back for the grouping
//! job and the best-file ranking.
//!
//! **The answer has a table of its own.** `file.quality_verdict` is the
//! read stage's verdict (`truncated` / `broken`, cleared by every read) and
//! an *input* to [`assess`]; nothing here writes it.
//!
//! **What the inputs are.** One query builds every [`Facts`] from the
//! columns [`INPUTS`] names; the same fragment builds the key stored with
//! each row, so what was read and what is checked for staleness can't
//! drift apart. A test fails if `file_quality` gets a column that is
//! neither in the key nor on its ignore list.
//!
//! **Staleness.** A row is current only while its `rules_version` is
//! [`RULES_VERSION`] and its key equals the key of the inputs now (a file
//! with no row has no key, so it never does). Anything
//! that changes an input (a new measurement, a re-read's codec, bitrate or
//! sample rate, a new audio hash, a changed read-stage verdict) therefore
//! makes it stale with no one having to say so, and a file with no row is
//! stale too. [`settle`] recomputes the stale ones; it opens no audio.
//!
//! **When it runs.** The quality job settles every stale file when it
//! starts (files that never get a measurement included: their headers
//! alone can flag them), and the file it just measured when a measurement
//! lands. Whatever acts on verdicts settles the files it reads first, so
//! the verdicts are current whether or not a quality job has run since the
//! inputs changed: [`settle_files`] for the files in hand (a lookup per
//! file), or [`settle`] for all of them (one query that reads every file
//! when nothing is stale). The grouping job (1bC-6) does the first for
//! every file it decides or makes due, and the second only at the start of
//! a deciding job that has a file to decide. A stale verdict alone
//! queues no job ([`super::request`]): that would add a job to every walk
//! of files nobody has read yet, to store rows that say nothing.
//!
//! **Reading** ([`stored_verdict`], [`stored_verdicts`]) gives the row only
//! while it's current. A file with no row, a stale row, or a row this build
//! can't read has *no verdict*: [`Assessment::verdict`] is `None`, never
//! `ok`.
//!
//! **What the stored strings don't give.** `file.codec` names this build
//! doesn't know (a later version's codec, a hand-edited row) map to no
//! codec, which the rules treat like [`Codec::Other`]: no bitrate level and
//! no transcode check. `file.quality_verdict` values other than `truncated`
//! and `broken` (`ok`, `low_bitrate`, `suspect_transcode`, `clipped`) were
//! never written by the read stage and are ignored as inputs. The key
//! covers `file_quality.error_kinds`, which the rules don't read; a change
//! there only costs one recomputation.
//!
//! **Rules version.** Bump [`RULES_VERSION`] whenever [`assess`]'s rules or
//! thresholds change: every file's verdict is then recomputed from the
//! stored measurements, without decoding audio again. (Measurements
//! themselves have their own [`super::VERSION`]; its rows are part of the
//! key.) A test pins the rules' output over a fixed set of inputs next to
//! the version, so changing a threshold without bumping it fails.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::ledger::Stored;
use super::verdict::{assess, Assessment, Facts, Finding, Verdict};
use crate::read::{Codec, Verdict as ReadVerdict};

/// The version of [`assess`]'s rules and thresholds. Bump it when either
/// changes, and every stored verdict is recomputed.
pub const RULES_VERSION: u8 = 1;

/// How many files a settle works on in one transaction.
const PAGE: usize = 500;
/// How many ids one read query is given.
pub(crate) const CHUNK: usize = 500;

/// The key over every input of [`assess`], as one SQL expression over the
/// aliases `f` (`file`) and `q` (`file_quality`, possibly absent). Each
/// value goes through `quote()`, so NULL (`NULL`), an empty string (`''`)
/// and an empty blob (`X''`) stay different, and the array is never NULL.
///
/// `file.quality_verdict` enters as the read stage's verdict only, as
/// [`read_verdict_of`] maps it. Every `file_quality` column is here except
/// `measured_at` ([`KEY_IGNORES`]).
pub(crate) const INPUTS: &str = "json_array(
        quote(f.audio_hash), quote(f.codec), quote(f.bitrate), quote(f.sample_rate),
        quote(CASE WHEN f.quality_verdict IN ('truncated', 'broken')
                   THEN f.quality_verdict END),
        quote(q.file_id), quote(q.audio_hash), quote(q.method_version),
        quote(q.cutoff_hz), quote(q.cutoff_gap), quote(q.decoded_ms), quote(q.header_ms),
        quote(q.decode_errors), quote(q.error_kinds), quote(q.ended), quote(q.failure))";

/// The `file_quality` columns the key leaves out on purpose: when a row was
/// written says nothing about what it measured.
#[cfg(test)]
pub(crate) const KEY_IGNORES: &[&str] = &["measured_at"];

/// `v`'s row is missing or not current for the inputs `f` and `q` have now:
/// made by other rules, or from other inputs. A missing row needs no arm of
/// its own: its `inputs` is NULL, which is never the key (`IS NOT` is true).
pub(crate) fn stale_condition() -> String {
    format!("(v.rules_version != {RULES_VERSION} OR v.inputs IS NOT {INPUTS})")
}

/// `ids` as the JSON array the queries read with `json_each`.
fn ids_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// The codec `file.codec` names, or `None` for NULL or a name this build
/// doesn't know. Names are the ones [`Codec::as_str`] writes.
pub(crate) fn codec_of(name: Option<&str>) -> Option<Codec> {
    let name = name?;
    ALL_CODECS.iter().copied().find(|c| c.as_str() == name)
}

/// Every codec, so a stored name can be found. A test fails if a codec is
/// missing from here.
pub(crate) const ALL_CODECS: [Codec; 20] = [
    Codec::Mp3,
    Codec::Pcm,
    Codec::PcmFloat,
    Codec::AdpcmMs,
    Codec::AdpcmIma,
    Codec::Alaw,
    Codec::Mulaw,
    Codec::Flac,
    Codec::Aac,
    Codec::Alac,
    Codec::Vorbis,
    Codec::Opus,
    Codec::Speex,
    Codec::Ac3,
    Codec::Eac3,
    Codec::Wma,
    Codec::WavPack,
    Codec::Musepack,
    Codec::Ape,
    Codec::Other,
];

/// What the read stage said, from `file.quality_verdict`: only its own two
/// verdicts. The column's other values are not the read stage's.
pub(crate) fn read_verdict_of(name: Option<&str>) -> Option<ReadVerdict> {
    match name? {
        "truncated" => Some(ReadVerdict::Truncated),
        "broken" => Some(ReadVerdict::Broken),
        _ => None,
    }
}

/// The verdict a stored name stands for.
fn verdict_of(name: &str) -> Option<Verdict> {
    [
        Verdict::Ok,
        Verdict::LowBitrate,
        Verdict::SuspectTranscode,
        Verdict::Truncated,
        Verdict::Broken,
    ]
    .into_iter()
    .find(|v| v.as_str() == name)
}

/// An assessment that says nothing: the file has no verdict yet.
fn none() -> Assessment {
    Assessment {
        verdict: None,
        findings: Vec::new(),
    }
}

/// One file's inputs, as the database holds them now.
struct Inputs {
    id: i64,
    codec: Option<String>,
    bitrate: Option<i64>,
    sample_rate: Option<i64>,
    quality_verdict: Option<String>,
    audio_hash: Option<Vec<u8>>,
    /// `None` when the file has no `file_quality` row.
    stored: Option<Stored>,
    /// The key [`INPUTS`] gives, to store with the answer.
    key: String,
    /// The verdict stored now, if a row exists (current or not).
    old: Option<String>,
}

impl Inputs {
    fn assess(&self) -> Assessment {
        assess(&Facts {
            codec: codec_of(self.codec.as_deref()),
            bitrate_kbps: self.bitrate,
            sample_rate_hz: self.sample_rate,
            read: read_verdict_of(self.quality_verdict.as_deref()),
            audio_hash: self.audio_hash.as_deref(),
            stored: self.stored.as_ref(),
        })
    }
}

/// What one [`settle_page`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Page {
    /// How many files' stored verdict is now a different one (a first
    /// verdict counts; a first row that says "none" doesn't).
    pub changed: usize,
    /// The highest file id the page covered; `None` when there was
    /// nothing stale after the cursor.
    pub last: Option<i64>,
}

/// The statement that lists one page of stale files with their inputs:
/// `?1` is the id to start after, `?2` the ids to look among as a JSON array
/// (NULL when every file is looked at), `?3` the page's size.
///
/// Two statements, because one can't serve both: with the ids given, each
/// file is looked up by its primary key, so settling one file costs one
/// lookup however many files there are (the quality job settles each file
/// it measures). A single statement with "`?2` is NULL or the id is among
/// them" walks the whole table either way. The `+` keeps the cursor from
/// being taken for the way in: it is only a filter there.
pub(crate) fn stale_page_sql(named: bool) -> String {
    let among = match named {
        true => "f.id IN (SELECT value FROM json_each(?2)) AND +f.id > ?1",
        // `?2` is bound (to NULL) and not read.
        false => "f.id > ?1",
    };
    format!(
        "SELECT f.id, f.codec, f.bitrate, f.sample_rate, f.quality_verdict, f.audio_hash,
                q.audio_hash, q.method_version, q.cutoff_hz, q.cutoff_gap, q.decoded_ms,
                q.header_ms, q.decode_errors, q.error_kinds, q.ended, q.failure,
                {INPUTS}, v.verdict
         FROM file f
         LEFT JOIN file_quality q ON q.file_id = f.id
         LEFT JOIN file_verdict v ON v.file_id = f.id
         WHERE {among}
           AND {stale}
         ORDER BY f.id LIMIT ?3",
        stale = stale_condition()
    )
}

/// Recomputes up to one page of stale files with an id above `after`
/// (among `only`, if given), in one transaction. The job calls it in a loop
/// on the writer, so a long settle can be cancelled and never holds the
/// writer for long.
pub(crate) fn settle_page(
    conn: &mut Connection,
    only: Option<&[i64]>,
    after: i64,
) -> rusqlite::Result<Page> {
    let only = only.map(ids_json);
    let tx = conn.transaction()?;
    let files = {
        let mut stmt = tx.prepare_cached(&stale_page_sql(only.is_some()))?;
        let rows = stmt.query_map(params![after, only, PAGE as i64], |r| {
            // The measurement is there when its (NOT NULL) audio hash is.
            let measured: Option<Vec<u8>> = r.get(6)?;
            let stored = match measured {
                Some(audio_hash) => Some(Stored {
                    audio_hash,
                    method_version: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    cutoff_hz: r.get(8)?,
                    cutoff_gap: r.get(9)?,
                    decoded_ms: r.get(10)?,
                    header_ms: r.get(11)?,
                    decode_errors: r.get::<_, Option<i64>>(12)?.unwrap_or(0),
                    error_kinds: r.get(13)?,
                    ended: r.get(14)?,
                    failure: r.get(15)?,
                }),
                None => None,
            };
            Ok(Inputs {
                id: r.get(0)?,
                codec: r.get(1)?,
                bitrate: r.get(2)?,
                sample_rate: r.get(3)?,
                quality_verdict: r.get(4)?,
                audio_hash: r.get(5)?,
                stored,
                key: r.get(16)?,
                old: r.get(17)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let Some(last) = files.last().map(|f| f.id) else {
        return Ok(Page {
            changed: 0,
            last: None,
        });
    };
    let mut changed = 0;
    {
        let mut upsert = tx.prepare_cached(
            "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict, findings)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (file_id) DO UPDATE SET
                 rules_version = excluded.rules_version, inputs = excluded.inputs,
                 verdict = excluded.verdict, findings = excluded.findings,
                 assessed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )?;
        for file in &files {
            let answer = file.assess();
            let verdict = answer.verdict.map(Verdict::as_str);
            if file.old.as_deref() != verdict {
                changed += 1;
            }
            let findings = serde_json::to_string(&answer.findings)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            upsert.execute(params![
                file.id,
                i64::from(RULES_VERSION),
                file.key,
                verdict,
                findings
            ])?;
        }
    }
    tx.commit()?;
    Ok(Page {
        changed,
        last: Some(last),
    })
}

/// Recomputes every stale file's verdict (see the module doc) and returns
/// how many stored verdicts came out different: a file's first verdict
/// counts, and a file assessed as having none doesn't. Opens no audio.
pub fn settle(conn: &mut Connection) -> rusqlite::Result<usize> {
    settle_among(conn, None)
}

/// [`settle`] for the files `ids` only (a file that isn't stale is left
/// alone).
pub fn settle_files(conn: &mut Connection, ids: &[i64]) -> rusqlite::Result<usize> {
    settle_among(conn, Some(ids))
}

fn settle_among(conn: &mut Connection, only: Option<&[i64]>) -> rusqlite::Result<usize> {
    let (mut changed, mut after) = (0, 0);
    loop {
        let page = settle_page(conn, only, after)?;
        changed += page.changed;
        match page.last {
            Some(last) => after = last,
            None => return Ok(changed),
        }
    }
}

/// Whether any file's verdict is missing or stale; among `only` if given.
pub fn any_stale(conn: &Connection, only: Option<&[i64]>) -> rusqlite::Result<bool> {
    let only = only.map(ids_json);
    conn.prepare_cached(&format!(
        "SELECT EXISTS (
             SELECT 1 FROM file f
             LEFT JOIN file_quality q ON q.file_id = f.id
             LEFT JOIN file_verdict v ON v.file_id = f.id
             WHERE (?1 IS NULL OR f.id IN (SELECT value FROM json_each(?1)))
               AND {stale})",
        stale = stale_condition()
    ))?
    .query_row([only], |r| r.get(0))
}

/// The verdict and findings stored for file `id`, as of the inputs it has
/// now. A file with none yet reads as no verdict ([`Assessment::verdict`]
/// `None`): never `ok`.
pub fn stored_verdict(conn: &Connection, id: i64) -> rusqlite::Result<Assessment> {
    let mut found = stored_verdicts(conn, &[id])?;
    Ok(found.remove(&id).unwrap_or_else(none))
}

/// [`stored_verdict`] for many files at once. Every id asked for is in the
/// map.
pub fn stored_verdicts(
    conn: &Connection,
    ids: &[i64],
) -> rusqlite::Result<HashMap<i64, Assessment>> {
    let mut found: HashMap<i64, Assessment> = ids.iter().map(|id| (*id, none())).collect();
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT v.file_id, v.verdict, v.findings
         FROM file_verdict v
         JOIN file f ON f.id = v.file_id
         LEFT JOIN file_quality q ON q.file_id = f.id
         WHERE v.file_id IN (SELECT value FROM json_each(?1))
           AND v.rules_version = {RULES_VERSION}
           AND v.inputs IS {INPUTS}"
    ))?;
    for chunk in ids.chunks(CHUNK) {
        let rows = stmt.query_map([ids_json(chunk)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, verdict, findings) = row?;
            // A row this build can't read is as good as none.
            let verdict = match verdict.as_deref() {
                None => Some(None),
                Some(name) => verdict_of(name).map(Some),
            };
            let findings = serde_json::from_str::<Vec<Finding>>(&findings);
            if let (Some(verdict), Ok(findings)) = (verdict, findings) {
                found.insert(id, Assessment { verdict, findings });
            }
        }
    }
    Ok(found)
}
