//! The import of rekordbox's My Tags (3A-T; ROADMAP 3.1: "Import first,
//! don't start empty"): a one-to-one copy of the names in
//! `rekordbox_track.my_tags` into tags, as ONE operation (one Undo).
//!
//! - **One to one.** Each My Tag name becomes one tag, linked by
//!   `tag.rekordbox_name`, spelled as rekordbox spells it. rekordbox's XML
//!   has no My Tag groups, so new tags go in one group: the group of the
//!   earlier import, else a group called what the caller says.
//! - **A tag's tracks are copied once**, when the tag is first imported, for
//!   the rekordbox tracks matched to a track. A later import never puts a tag
//!   back on a track the user took it off.
//! - **A My Tag that disappears from rekordbox stays**, flagged
//!   (`gone_from_rekordbox`); it is never deleted. It comes back unflagged if
//!   a later read has it again. Nothing is flagged while the snapshot is
//!   empty: no read says nothing about what rekordbox has.
//! - **A tag the user made with the same name** (by the writer's sibling
//!   rule) and that isn't linked yet is linked, not duplicated.
//! - **Bad data never stops the import.** A name the writer can't carry, or
//!   one that equals another My Tag's, is skipped and counted.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;

use super::{
    make_group_in, make_tag_in, names_in, sendable, trimmed, Done, TagGroupId, IMPORT_OPERATION,
};
use crate::ops::{self, OpsError};
use crate::rekordbox_write::sibling_key;

/// What an import did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagsImported {
    /// Tags made, or linked to a My Tag.
    pub added: u32,
    /// Tags newly flagged as gone from rekordbox.
    pub gone: u32,
    /// Flagged tags that rekordbox has again.
    pub back: u32,
    /// Tracks given a tag.
    pub applied: u32,
    /// My Tags left out: a name the writer can't carry, or one equal to
    /// another My Tag's.
    pub skipped: u32,
    /// The operation this recorded; `None` when nothing changed.
    pub operation_id: Option<i64>,
}

/// The group new tags go in.
fn group_for(rec: &mut ops::Recorder<'_>, name: &str) -> Result<Done<TagGroupId>, OpsError> {
    let earlier: Vec<i64> = rec.read_rows(
        "SELECT group_id FROM tag WHERE rekordbox_name IS NOT NULL ORDER BY id LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    if let Some(&id) = earlier.first() {
        return Ok(Ok(TagGroupId(id)));
    }
    let key = sibling_key(name.trim());
    if let Some((id, _)) = names_in(rec, "tag_group", None)?
        .into_iter()
        .find(|(_, other)| sibling_key(other) == key)
    {
        return Ok(Ok(TagGroupId(id)));
    }
    make_group_in(rec, name)
}

/// Copies rekordbox's My Tags into the vocabulary; see the module notes.
pub fn import_on(conn: &mut Connection, group_name: &str) -> Result<Done<TagsImported>, OpsError> {
    // Checked first, so no refusal can come after a write.
    let group_name = match trimmed(group_name) {
        Ok(name) => name,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let group_name = group_name.as_str();
    let recorded = ops::record(conn, IMPORT_OPERATION, &serde_json::json!({}), |rec| {
        // Each My Tag name, with the tracks rekordbox has it on that are
        // matched to a track. A name on an unmatched track still counts.
        let mut wanted: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        let rows = rec.read_rows(
            "SELECT j.value, rt.recording_id
             FROM rekordbox_track rt, json_each(rt.my_tags) j
             WHERE j.type = 'text'
               AND rt.read_at = (SELECT max(read_at) FROM rekordbox_track)
             ORDER BY rt.id",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
        )?;
        for (name, recording) in rows {
            let name = name.trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let tracks = wanted.entry(name).or_default();
            if let Some(recording) = recording {
                if !tracks.contains(&recording) {
                    tracks.push(recording);
                }
            }
        }
        let snapshot_has_tracks = !rec
            .read_rows(
                "SELECT 1 FROM rekordbox_track
                 WHERE read_at = (SELECT max(read_at) FROM rekordbox_track) LIMIT 1",
                [],
                |r| r.get::<_, i64>(0),
            )?
            .is_empty();

        // rekordbox name -> (tag, flagged gone); sibling key -> (tag, linked).
        let mut linked: HashMap<String, (i64, bool)> = HashMap::new();
        let mut by_key: HashMap<String, (i64, bool)> = HashMap::new();
        for (id, name, from, gone) in rec.read_rows(
            "SELECT id, name, rekordbox_name, gone_from_rekordbox FROM tag",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            },
        )? {
            by_key.insert(sibling_key(&name), (id, from.is_some()));
            if let Some(from) = from {
                linked.insert(from, (id, gone));
            }
        }

        let mut done = TagsImported::default();
        let mut group: Option<TagGroupId> = None;
        for (name, tracks) in &wanted {
            if let Some(&(id, gone)) = linked.get(name) {
                if gone {
                    rec.set("tag", id, "gone_from_rekordbox", 0)?;
                    done.back += 1;
                }
                continue;
            }
            let key = sibling_key(name);
            let (tag, already): (i64, HashSet<i64>) = match by_key.get(&key) {
                // A tag of the user's with this name: link it.
                Some(&(id, false)) => {
                    let has = rec.read_rows(
                        "SELECT recording_id FROM tag_applied WHERE tag_id = ?1",
                        [id],
                        |r| r.get(0),
                    )?;
                    rec.set("tag", id, "rekordbox_name", name.clone())?;
                    (id, has.into_iter().collect())
                }
                // Another My Tag's name, as the writer compares names.
                Some(&(_, true)) => {
                    done.skipped += 1;
                    continue;
                }
                None => {
                    if !sendable(name) {
                        done.skipped += 1;
                        continue;
                    }
                    let in_group = match group {
                        Some(group) => group,
                        None => match group_for(rec, group_name)? {
                            Ok(found) => *group.insert(found),
                            Err(refusal) => return Ok(Err(refusal)),
                        },
                    };
                    match make_tag_in(rec, in_group, name, Some(name))? {
                        Ok(id) => (id.0, HashSet::new()),
                        Err(_) => {
                            done.skipped += 1;
                            continue;
                        }
                    }
                }
            };
            by_key.insert(key, (tag, true));
            done.added += 1;
            for &recording in tracks.iter().filter(|r| !already.contains(r)) {
                rec.insert(
                    "tag_applied",
                    &[("recording_id", recording.into()), ("tag_id", tag.into())],
                )?;
                done.applied += 1;
            }
        }
        if snapshot_has_tracks {
            for (name, &(id, gone)) in &linked {
                if !gone && !wanted.contains_key(name) {
                    rec.set("tag", id, "gone_from_rekordbox", 1)?;
                    done.gone += 1;
                }
            }
        }
        rec.detail("tags", done.added);
        Ok(Ok(done))
    })?;
    Ok(recorded.value.map(|done| TagsImported {
        operation_id: recorded.operation_id,
        ..done
    }))
}
