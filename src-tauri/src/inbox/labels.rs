//! What each Inbox file is to the user: its label and the reason for it
//! (ROADMAP 2.1).
//!
//! Nothing is stored. A label is worked out when the Inbox is read, from
//! what the app knows by then, so it follows the Library as it changes:
//!
//! | label | when |
//! |---|---|
//! | `checking` | grouping hasn't decided the file yet (fingerprint, matching, deciding), the file has no track yet, or it is online-only and unread |
//! | `already_have` | its track is in the Library and the file is not ahead of the track's linked file |
//! | `better_file` | its track is in the Library and the best-file ranking puts it ahead of the linked file |
//! | `other_cut` | its track isn't in the Library, but a cut link joins it to one that is |
//! | `rework` | the same with a rework link |
//! | `new_to_you` | none of the above: a decided file whose track and links lead nowhere in the Library |
//!
//! (`on_wishlist` comes with the wishlist, 2.5: the enum has room, nothing
//! more.)
//!
//! The reason is a [`Reason`]: an i18n key (`inbox:reason.<code>`; a better
//! file's reasons are the Review screen's, `review:reason.*`) with
//! parameters that are data (a codec's name, a link's label), never words.
//! The words are in the locale files.
//!
//! Reading only, and it never fails a page: a file with a missing row, a
//! NULL where one isn't expected or no track gets `checking`, and a failure
//! to load the ranking's inputs does the same for the files that needed it.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::best_file::{why_ahead, Reason as Rank};
use crate::file_roles::facts::{self, Loaded};
use crate::review::reasons::from_rank;
use crate::suggest::Reason;

/// What an Inbox file is to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    /// Not through fingerprint, matching and deciding yet.
    Checking,
    /// Its track isn't in the Library, and no version link leads to one.
    NewToYou,
    /// Its track is in the Library, and this file adds nothing.
    AlreadyHave,
    /// Its track is in the Library, and this file ranks above the linked one.
    BetterFile,
    /// Another cut of a track in the Library (Radio Edit of your Extended).
    OtherCut,
    /// A rework of a track in the Library.
    Rework,
}

/// The i18n key of each reason that isn't the ranking's.
pub mod keys {
    pub const CHECKING: &str = "inbox:reason.checking";
    pub const CHECKING_ONLINE_ONLY: &str = "inbox:reason.checkingOnlineOnly";
    pub const NEW_TO_YOU: &str = "inbox:reason.newToYou";
    pub const ALREADY_HAVE: &str = "inbox:reason.alreadyHave";
    /// Parameter `label`: the link's label ("radio"), when it has one.
    pub const OTHER_CUT: &str = "inbox:reason.otherCut";
    /// Parameter `label`: the link's label ("remix"), when it has one.
    pub const REWORK: &str = "inbox:reason.rework";
}

/// A file's label with its reason.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Labeled {
    pub label: Label,
    pub reason: Reason,
    /// The Library track the label is about: for `already_have` and
    /// `better_file` the track the file belongs to, for `other_cut` and
    /// `rework` the track it is a cut or rework of.
    pub library_track_id: Option<i64>,
}

fn fact(key: &str) -> Reason {
    Reason::fact(key).unwrap_or_else(|e| panic!("bad reason key {key}: {e}"))
}

impl Labeled {
    fn new(label: Label, reason: Reason, library_track_id: Option<i64>) -> Labeled {
        Labeled {
            label,
            reason,
            library_track_id,
        }
    }

    /// The label of a file nothing is known about yet.
    pub fn checking() -> Labeled {
        Labeled::new(Label::Checking, fact(keys::CHECKING), None)
    }

    fn linked(label: Label, key: &str, link_label: Option<String>, track: i64) -> Labeled {
        let mut reason = fact(key);
        if let Some(name) = link_label.filter(|name| !name.is_empty()) {
            if let Ok(with) = reason.clone().with_text("label", name) {
                reason = with;
            }
        }
        Labeled::new(label, reason, Some(track))
    }
}

/// What the database says about one file.
struct Facts {
    file: i64,
    online_only: bool,
    decided: bool,
    recording: Option<i64>,
    library_track: Option<i64>,
    linked_file: Option<i64>,
}

/// The label of each of `files`. A file that isn't a `file` row isn't in the
/// result (the caller treats it as [`Labeled::checking`]).
pub fn label_files(conn: &Connection, files: &[i64]) -> rusqlite::Result<HashMap<i64, Labeled>> {
    let mut out = HashMap::new();
    if files.is_empty() {
        return Ok(out);
    }
    let wanted = serde_json::to_string(files).unwrap_or_else(|_| "[]".into());
    let found = read_facts(conn, &wanted)?;
    // The recordings whose files must be ranked (in the Library) and those
    // whose links must be looked at (decided, not in the Library).
    let mut to_rank: Vec<i64> = Vec::new();
    let mut to_link: Vec<i64> = Vec::new();
    for f in &found {
        match (f.decided, f.recording, f.library_track) {
            (true, Some(rec), Some(_)) if f.linked_file.is_some_and(|l| l != f.file) => {
                to_rank.push(rec)
            }
            (true, Some(rec), None) => to_link.push(rec),
            _ => {}
        }
    }
    to_rank.sort_unstable();
    to_rank.dedup();
    to_link.sort_unstable();
    to_link.dedup();
    let ranked = if to_rank.is_empty() {
        Some(BTreeMap::new())
    } else {
        facts::load(conn, &to_rank).ok()
    };
    // Unreadable links are `None`: a file that would need them is `checking`.
    let links = links_to_library(conn, &to_link).ok();

    for f in found {
        let labeled = if !f.decided || f.recording.is_none() {
            Labeled::new(
                Label::Checking,
                fact(if f.online_only {
                    keys::CHECKING_ONLINE_ONLY
                } else {
                    keys::CHECKING
                }),
                None,
            )
        } else if let (Some(rec), Some(track)) = (f.recording, f.library_track) {
            in_library(f.file, rec, track, f.linked_file, ranked.as_ref())
        } else if links.is_none() {
            Labeled::checking()
        } else if let Some(link) = f
            .recording
            .and_then(|rec| links.as_ref().and_then(|l| l.get(&rec)))
        {
            let (label, key) = if link.cut {
                (Label::OtherCut, keys::OTHER_CUT)
            } else {
                (Label::Rework, keys::REWORK)
            };
            Labeled::linked(label, key, link.label.clone(), link.track)
        } else {
            Labeled::new(Label::NewToYou, fact(keys::NEW_TO_YOU), None)
        };
        out.insert(f.file, labeled);
    }
    Ok(out)
}

/// The label of a file whose track is in the Library as `track`.
fn in_library(
    file: i64,
    recording: i64,
    track: i64,
    linked_file: Option<i64>,
    ranked: Option<&BTreeMap<i64, Vec<Loaded>>>,
) -> Labeled {
    let already = || Labeled::new(Label::AlreadyHave, fact(keys::ALREADY_HAVE), Some(track));
    // The track's linked file is the file this one is held against. With
    // none (a missing file) or this very file, there is nothing to beat.
    let Some(linked) = linked_file.filter(|l| *l != file) else {
        return already();
    };
    // The ranking's inputs couldn't be read: say so rather than guess.
    let Some(ranked) = ranked else {
        return Labeled::checking();
    };
    let files = ranked.get(&recording).map(Vec::as_slice).unwrap_or(&[]);
    let find = |id: i64| files.iter().find(|l| l.facts.file_id == id);
    let (Some(this), Some(other)) = (find(file), find(linked)) else {
        return already();
    };
    match why_ahead(&this.facts, &other.facts) {
        // A tie goes to the lower file id; it isn't a better file.
        Some(Rank::Tied) | None => already(),
        Some(reason) => Labeled::new(Label::BetterFile, from_rank(&reason), Some(track)),
    }
}

fn read_facts(conn: &Connection, wanted: &str) -> rusqlite::Result<Vec<Facts>> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.id, f.online_only, g.decided_as IS NOT NULL, rf.recording_id,
                lt.id, lt.linked_file_id
         FROM file f
         LEFT JOIN grouping_file g ON g.file_id = f.id
         LEFT JOIN recording_file rf ON rf.file_id = f.id
         LEFT JOIN library_track lt ON lt.recording_id = rf.recording_id
         WHERE f.id IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = stmt.query_map([wanted], |r| {
        Ok(Facts {
            file: r.get(0)?,
            online_only: r.get::<_, Option<bool>>(1)?.unwrap_or(false),
            decided: r.get::<_, Option<bool>>(2)?.unwrap_or(false),
            recording: r.get(3)?,
            library_track: r.get(4)?,
            linked_file: r.get(5)?,
        })
    })?;
    // A row that can't be read is left out: its file stays `checking`.
    Ok(rows.filter_map(Result::ok).collect())
}

/// A version link from a recording to one in the Library.
struct Link {
    cut: bool,
    label: Option<String>,
    track: i64,
}

/// For each of `recordings`, the link to a Library track that says most: a
/// cut before a rework (the same production), a confirmed link before an
/// unconfirmed one, then the oldest.
fn links_to_library(conn: &Connection, recordings: &[i64]) -> rusqlite::Result<HashMap<i64, Link>> {
    let wanted = serde_json::to_string(recordings).unwrap_or_else(|_| "[]".into());
    let mut stmt = conn.prepare_cached(
        "SELECT r.value, v.kind, v.label, lt.id
         FROM json_each(?1) r
         JOIN version_link v ON v.recording_a = r.value OR v.recording_b = r.value
         JOIN library_track lt ON lt.recording_id =
              CASE WHEN v.recording_a = r.value THEN v.recording_b ELSE v.recording_a END
         ORDER BY (v.kind = 'cut') DESC, v.confirmed DESC, v.id",
    )?;
    let rows = stmt.query_map([wanted], |r| {
        let kind: String = r.get(1)?;
        Ok((
            r.get::<_, i64>(0)?,
            Link {
                cut: kind == "cut",
                label: r.get(2)?,
                track: r.get(3)?,
            },
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (recording, link) = row?;
        out.entry(recording).or_insert(link);
    }
    Ok(out)
}
