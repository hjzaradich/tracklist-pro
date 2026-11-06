//! Auto tick-off (ROADMAP 2.5): a new Inbox file that is a wishlist item,
//! the very same version, is added to the Library for you, and the item is
//! marked acquired, as ONE operation. Undo takes all of it back: the file
//! is a new Inbox item again, the item is wanted again, and the crate entry
//! (and a crate the tick-off made) goes too.
//!
//! # When
//!
//! After the hash job's intake and after each grouping job: a file is only
//! in a track, and so matchable, once grouping has decided it. A pass looks
//! at Inbox items that are `new`, arrived after the folder's first look
//! (not `already_there`), on disk, in a decided track.
//!
//! # What it does, and doesn't
//!
//! - Owned only: another cut or a rework of the item is never ticked off.
//! - One file for one item. A file that answers two items, or an item two
//!   files answer, is left to the user.
//! - The Inbox's own "Add to Library" rules decide ([`add_check`]); a file
//!   with a warning to agree to (low bitrate) is never added for you. A
//!   refused file stays in the Inbox, and the Done for you group shows the
//!   match with the reason.
//! - An item anything was ever logged on (the user set it aside and put it
//!   back, or undid a tick-off) is never ticked off: the log is the mark,
//!   no column is needed.
//! - Optional, per wishlist and off by default: the track goes into the
//!   crate named after the wishlist, the one there is when the name is
//!   taken. Stored as a setting (`wishlist_crate_<id>`), like the switch.
//!
//! Bad data in one file never stops the others: each tick-off is its own
//! operation, and a failure is logged and skipped. Nothing while wishlists
//! are off.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use super::matching::Matcher;
use super::{enabled, wishlist_exists, WishFor, WishTrack, WishlistOutcome, WishlistRefusal};
use crate::db::{ReadPool, Writer};
use crate::inbox::actions::{add_check, add_in, ask_after_add, InboxRefusal};
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::ops::{self, OpsError};
use crate::suggest::Reason;

#[cfg(test)]
#[path = "tick_off_tests.rs"]
mod tests;

/// The operation kind, as the log stores it.
pub const OPERATION: &str = "wishlist_tick_off";

/// The most "Done for you" entries the Inbox shows of each kind, newest
/// first.
pub const SHOWN: i64 = 100;

/// A file the tick-off looks at: its Inbox item and its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Arrival {
    item: i64,
    file: i64,
    recording: i64,
}

/// What a file answers.
#[derive(Debug, Clone, PartialEq)]
enum Answer {
    One(WishFor),
    /// Two items or more, or an item another file answers too.
    Several,
}

/// The new Inbox files in a decided track. A row that can't be read is
/// left out.
fn arrivals(conn: &Connection) -> rusqlite::Result<Vec<Arrival>> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.id, i.file_id, rf.recording_id
         FROM inbox_item i
         JOIN inbox_batch b ON b.id = i.batch_id
         JOIN file f ON f.id = i.file_id
         JOIN recording_file rf ON rf.file_id = f.id
         JOIN grouping_file g ON g.file_id = f.id
         WHERE i.status = 'new' AND b.kind <> 'already_there'
           AND f.present = 1 AND f.hidden_reason IS NULL
           AND g.decided_as IS NOT NULL
         ORDER BY i.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Arrival {
            item: r.get(0)?,
            file: r.get(1)?,
            recording: r.get(2)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// The wish items not acquired yet, on every list. A row that can't be read
/// is left out.
fn open_wishes(conn: &Connection) -> rusqlite::Result<Vec<(i64, i64, WishTrack)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, wishlist_id, raw_artist, raw_title, raw_version, raw_isrc
         FROM wish_item
         WHERE acquired_at IS NULL AND acquired_file_id IS NULL
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            WishTrack {
                artist: r.get(2)?,
                title: r.get(3)?,
                version: r.get(4)?,
                isrc: r.get(5)?,
            },
        ))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// What each of `files` (with its track) answers: one matcher over their
/// tracks, one look at each open item.
fn answers(conn: &Connection, files: &[(i64, i64)]) -> rusqlite::Result<HashMap<i64, Answer>> {
    let mut out = HashMap::new();
    if files.is_empty() {
        return Ok(out);
    }
    let mut files_of: HashMap<i64, Vec<i64>> = HashMap::new();
    for &(file, recording) in files {
        let of = files_of.entry(recording).or_default();
        if !of.contains(&file) {
            of.push(file);
        }
    }
    let recordings: Vec<i64> = files_of.keys().copied().collect();
    let matcher = Matcher::build_for(conn, &recordings)?;
    let mut found: HashMap<i64, Vec<WishFor>> = HashMap::new();
    let mut several: HashSet<i64> = HashSet::new();
    for (item, wishlist, track) in open_wishes(conn)? {
        let mut by: Vec<(i64, WishFor)> = Vec::new();
        for owner in matcher.owners(&track) {
            let of = owner.recording_id.and_then(|r| files_of.get(&r));
            for &file in of.into_iter().flatten() {
                let wish = WishFor {
                    wishlist_id: wishlist,
                    item_id: item,
                    result: owner.clone(),
                };
                by.push((file, wish));
            }
        }
        if by.len() > 1 {
            several.extend(by.iter().map(|(file, _)| *file));
        }
        for (file, wish) in by {
            found.entry(file).or_default().push(wish);
        }
    }
    for (file, mut wishes) in found {
        let answer = if wishes.len() == 1 && !several.contains(&file) {
            Answer::One(wishes.swap_remove(0))
        } else {
            Answer::Several
        };
        out.insert(file, answer);
    }
    Ok(out)
}

/// The wish item each of `files` answers, owned as the very same version,
/// for those that answer exactly one and no other new Inbox file answers it
/// too. One look at the arrivals and one at the open items for the whole
/// batch, however many files. Nothing while wishlists are off.
pub(super) fn wishes_for(
    conn: &Connection,
    files: &[i64],
) -> rusqlite::Result<HashMap<i64, WishFor>> {
    if files.is_empty() || !enabled(conn)? {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT rf.recording_id FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE f.id = ?1 AND f.present = 1 AND f.hidden_reason IS NULL",
    )?;
    let mut asked: Vec<(i64, i64)> = Vec::new();
    for &file in files {
        if let Some(recording) = stmt.query_row([file], |r| r.get::<_, i64>(0)).optional()? {
            asked.push((file, recording));
        }
    }
    let mut all: Vec<(i64, i64)> = arrivals(conn)?
        .into_iter()
        .map(|a| (a.file, a.recording))
        .collect();
    all.extend(asked.iter().copied());
    let mut answered = answers(conn, &all)?;
    let mut out = HashMap::new();
    for (file, _) in asked {
        if let Some(Answer::One(wish)) = answered.remove(&file) {
            out.insert(file, wish);
        }
    }
    Ok(out)
}

/// [`wishes_for`] for one file.
pub(super) fn wish_for(conn: &Connection, file: i64) -> rusqlite::Result<Option<WishFor>> {
    Ok(wishes_for(conn, &[file])?.remove(&file))
}

fn crate_key(wishlist: i64) -> String {
    format!("wishlist_crate_{wishlist}")
}

/// Whether ticked-off tracks of `wishlist` go into its crate. Off for no
/// row, or any value but `true`.
pub fn auto_crate(conn: &Connection, wishlist: i64) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT json_type(value) = 'true' FROM setting WHERE key = ?1",
            [crate_key(wishlist)],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Turns the crate of `wishlist` on or off. A setting, not an operation.
pub fn set_auto_crate(
    conn: &mut Connection,
    wishlist: i64,
    on: bool,
) -> rusqlite::Result<WishlistOutcome> {
    if !enabled(conn)? {
        return Ok(super::refused(WishlistRefusal::NotTurnedOn));
    }
    if !wishlist_exists(conn, wishlist)? {
        return Ok(super::refused(WishlistRefusal::NoSuchWishlist));
    }
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, json(?2))
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (crate_key(wishlist), if on { "true" } else { "false" }),
    )?;
    Ok(super::done(None, wishlist))
}

/// Whether the log ever recorded a change of Inbox `item`.
fn touched(conn: &Connection, item: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM change WHERE entity = 'inbox_item' AND entity_id = ?1)",
    )?
    .query_row([item], |r| r.get(0))
}

/// Ticks `wish` off with the file of Inbox `item`, as one operation, if
/// the Add rules let it. Whether it did.
fn tick_off(conn: &mut Connection, item: i64, wish: &WishFor) -> Result<bool, OpsError> {
    if touched(conn, item)? {
        return Ok(false);
    }
    let Ok((recording, file)) = add_check(conn, item)? else {
        return Ok(false);
    };
    if crate::library::warnings_for(conn, &[file])?.contains_key(&file) {
        return Ok(false);
    }
    let name: String = conn.query_row(
        "SELECT name FROM wishlist WHERE id = ?1",
        [wish.wishlist_id],
        |r| r.get(0),
    )?;
    let into_crate = auto_crate(conn, wish.wishlist_id)?;
    let details = json!({
        "items": [item],
        "wishItemId": wish.item_id,
        "wishlistId": wish.wishlist_id,
        "name": name,
    });
    let wished = wish.item_id;
    let recorded = ops::record(conn, OPERATION, &details, |rec| {
        let track = add_in(rec, item, recording, file)?;
        let now: String = rec
            .read_rows("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                r.get(0)
            })?
            .pop()
            .unwrap_or_default();
        rec.set("wish_item", wished, "acquired_file_id", file)?;
        rec.set("wish_item", wished, "acquired_at", now)?;
        if into_crate {
            if let Some((id, made)) = crate::crates::put_in_named(rec, &name, track)? {
                rec.detail("crateId", id.0);
                rec.detail("newCrate", made);
            }
        }
        Ok(())
    })?;
    Ok(recorded.operation_id.is_some())
}

/// What a pass looked at: the database, the arrivals, and marks that move
/// when the wishlists, the Library or the log do.
type Looked = (String, Vec<Arrival>, [i64; 6]);

/// What the last pass looked at, so a pass with nothing new to look at is
/// skipped. A match the Add rules refused is looked at again once one of
/// those moves.
static LAST_PASS: Mutex<Option<Looked>> = Mutex::new(None);

fn marks(conn: &Connection) -> rusqlite::Result<[i64; 6]> {
    conn.query_row(
        "SELECT (SELECT ifnull(max(id), 0) FROM wish_item),
                (SELECT count(*) FROM wish_item WHERE acquired_at IS NULL),
                (SELECT ifnull(max(id), 0) FROM operation),
                (SELECT count(*) FROM library_track),
                (SELECT count(*) FROM setting WHERE key LIKE 'wishlist_crate_%'),
                (SELECT count(*) FROM operation WHERE undone_at IS NOT NULL)",
        [],
        |r| {
            Ok([
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ])
        },
    )
}

/// One pass: ticks off what it can. How many it ticked off. Nothing while
/// wishlists are off.
pub fn run(conn: &mut Connection) -> rusqlite::Result<usize> {
    if !enabled(conn)? {
        return Ok(0);
    }
    let arrived = arrivals(conn)?;
    let seen = (
        conn.path().unwrap_or_default().to_owned(),
        arrived.clone(),
        marks(conn)?,
    );
    {
        let last = LAST_PASS.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(&seen) {
            return Ok(0);
        }
    }
    let files: Vec<(i64, i64)> = arrived.iter().map(|a| (a.file, a.recording)).collect();
    let answers = answers(conn, &files)?;
    let mut ticked = 0;
    for arrival in &arrived {
        let Some(Answer::One(wish)) = answers.get(&arrival.file) else {
            continue;
        };
        match tick_off(conn, arrival.item, wish) {
            Ok(true) => ticked += 1,
            Ok(false) => {}
            Err(e) => eprintln!(
                "wishlist tick-off: Inbox item {} skipped: {e}",
                arrival.item
            ),
        }
    }
    // What this pass changed moves the marks; the next pass looks again.
    let after = (seen.0, arrivals(conn)?, marks(conn)?);
    *LAST_PASS.lock().unwrap_or_else(|e| e.into_inner()) = Some(after);
    Ok(ticked)
}

/// A pass after `job`, and the background work an add asks for when it
/// ticked anything off. Best effort: a failure is logged, never the job's.
pub(crate) fn after_job(job: &JobContext) {
    let writer = job.writer();
    match writer.call(run) {
        Ok(0) => {}
        Ok(_) => {
            if let Err(e) = crate::search::request(writer, |j| job.enqueue(j)) {
                eprintln!("wishlist tick-off: couldn't ask for the index: {e:?}");
            }
            ask_after_add(writer, None, |j| job.enqueue(j));
        }
        Err(e) => eprintln!("wishlist tick-off failed: {e}"),
    }
}

/// `inner` (the grouping job), with a tick-off pass after it, whether it
/// finished or was stopped: the files it decided can be ticked off.
pub fn after<H: JobHandler>(inner: H) -> TickingOff<H> {
    TickingOff { inner }
}

/// See [`after`].
pub struct TickingOff<H> {
    inner: H,
}

impl<H: JobHandler> JobHandler for TickingOff<H> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let result = self.inner.run(job);
        after_job(job);
        result
    }
}

/// A file the tick-off added for you, or matched and left to you.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DoneForYou {
    pub inbox_item_id: i64,
    pub file_id: i64,
    /// The file's own name (no folders).
    pub file_name: String,
    pub wishlist_id: i64,
    pub wishlist_name: String,
    pub wish_item_id: i64,
    pub wish_artist: String,
    pub wish_title: String,
    pub wish_version: Option<String>,
    /// How it matched (a `wishlist:reason.*` key).
    pub reason: Reason,
    /// Added to the Library for you. False: matched, and left to you.
    pub added: bool,
    /// Why "Add to Library" refuses it, when it does. None on a match left
    /// to you: a warning to agree to, or a tick-off undone.
    pub refusal: Option<InboxRefusal>,
}

fn file_name(rel: &str) -> String {
    rel.rsplit(['/', '\\']).next().unwrap_or(rel).to_owned()
}

/// What a wish item and its list say, for an entry.
struct Row {
    item: i64,
    file: i64,
    rel: String,
    wishlist: i64,
    wishlist_name: String,
    wish: i64,
    track: WishTrack,
    recording: Option<i64>,
}

/// The columns [`row`] reads; `{file}` is the file's id.
const ROW: &str = "SELECT i.id, f.id, f.rel_path, w.id, w.name, wi.id,
                          wi.raw_artist, wi.raw_title, wi.raw_version, wi.raw_isrc,
                          rf.recording_id
                   FROM wish_item wi
                   JOIN wishlist w ON w.id = wi.wishlist_id
                   JOIN file f ON f.id = {file}
                   JOIN inbox_item i ON i.file_id = f.id
                   LEFT JOIN recording_file rf ON rf.file_id = f.id";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        item: r.get(0)?,
        file: r.get(1)?,
        rel: r.get(2)?,
        wishlist: r.get(3)?,
        wishlist_name: r.get(4)?,
        wish: r.get(5)?,
        track: WishTrack {
            artist: r.get(6)?,
            title: r.get(7)?,
            version: r.get(8)?,
            isrc: r.get(9)?,
        },
        recording: r.get(10)?,
    })
}

/// The Inbox's "Done for you" group: the files ticked off for you (still
/// added, newest first), then the new files that answer one item but were
/// left to you, each with how it matched. Empty while wishlists are off.
pub fn done_for_you(conn: &Connection) -> rusqlite::Result<Vec<DoneForYou>> {
    if !enabled(conn)? {
        return Ok(Vec::new());
    }
    let mut rows: Vec<(Row, Option<WishFor>)> = Vec::new();
    {
        let mut stmt = conn.prepare_cached(&format!(
            "{} WHERE i.status = 'added'
             ORDER BY wi.acquired_at DESC, wi.id DESC LIMIT ?1",
            ROW.replace("{file}", "wi.acquired_file_id")
        ))?;
        let found = stmt.query_map([SHOWN], row)?;
        rows.extend(found.filter_map(Result::ok).map(|r| (r, None)));
    }
    let arrived = arrivals(conn)?;
    let files: Vec<(i64, i64)> = arrived.iter().map(|a| (a.file, a.recording)).collect();
    let mut matched: Vec<(i64, WishFor)> = answers(conn, &files)?
        .into_iter()
        .filter_map(|(file, answer)| match answer {
            Answer::One(wish) => Some((file, wish)),
            Answer::Several => None,
        })
        .collect();
    matched.sort_by_key(|(file, _)| std::cmp::Reverse(*file));
    matched.truncate(SHOWN as usize);
    let mut stmt = conn.prepare_cached(&format!(
        "{} WHERE wi.id = ?1 AND i.status = 'new'",
        ROW.replace("{file}", "?2")
    ))?;
    for (file, wish) in matched {
        if let Some(found) = stmt.query_row((wish.item_id, file), row).optional()? {
            rows.push((found, Some(wish)));
        }
    }
    let recordings: Vec<i64> = rows.iter().filter_map(|(r, _)| r.recording).collect();
    let matcher = Matcher::build_for(conn, &recordings)?;
    let mut out = Vec::with_capacity(rows.len());
    for (r, wish) in rows {
        let added = wish.is_none();
        let reason = match wish {
            Some(wish) => wish.result.reason,
            None => {
                matcher
                    .owners(&r.track)
                    .into_iter()
                    .find(|m| m.recording_id == r.recording)
                    .unwrap_or_else(|| matcher.find(&r.track))
                    .reason
            }
        };
        let refusal = if added {
            None
        } else {
            add_check(conn, r.item)?.err()
        };
        out.push(DoneForYou {
            inbox_item_id: r.item,
            file_id: r.file,
            file_name: file_name(&r.rel),
            wishlist_id: r.wishlist,
            wishlist_name: r.wishlist_name,
            wish_item_id: r.wish,
            wish_artist: r.track.artist,
            wish_title: r.track.title,
            wish_version: r.track.version,
            reason,
            added,
            refusal,
        });
    }
    Ok(out)
}

/// The Inbox's "Done for you" group ([`done_for_you`]).
#[tauri::command]
#[specta::specta]
pub async fn inbox_done_for_you(reads: State<'_, ReadPool>) -> Result<Vec<DoneForYou>, IpcError> {
    Ok(reads.read(done_for_you)?)
}

/// Whether ticked-off tracks of a wishlist go into its crate.
#[tauri::command]
#[specta::specta]
pub async fn wishlist_auto_crate(
    reads: State<'_, ReadPool>,
    wishlist_id: i64,
) -> Result<bool, IpcError> {
    Ok(reads.read(move |c| auto_crate(c, wishlist_id))?)
}

/// Turns the crate of a wishlist on or off.
#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_set_auto_crate(
    writer: State<'_, Writer>,
    wishlist_id: i64,
    on: bool,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| set_auto_crate(c, wishlist_id, on))?)
}
