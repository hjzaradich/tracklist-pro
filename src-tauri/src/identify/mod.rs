//! Identification: suggestions for a Library track's details, and "Use"
//! (ROADMAP 2.2, owner decisions 2026-11-08; Run 6A, S1).
//!
//! # The fields
//!
//! By rekordbox attribute name ([`IdentifyField`]): `Name` (the title),
//! `Artist`, `Mix` (the version), `Remixer`, `Label`, `Year` and `Album`,
//! plus the catalogue number, which the app keeps but never sends
//! (rekordbox's XML has no attribute for it; table `catalogue_number`,
//! migration 0057). Not here: ISRC, BPM, key, genre (it has its own flow,
//! `genre::own`) and artwork.
//!
//! # Suggestions ([`suggestions`], command [`identify_suggestions`])
//!
//! Per field: the **current value**, which is what the next send would
//! carry for it (the app's own value, else what a send takes: rekordbox's
//! for a track it has, the files' tags for one it doesn't; read from what
//! is stored, no audio file is opened), and the **suggestions**, each a
//! value with every source that gave it ([`IdentifySource`]). The rules
//! (`rank`):
//!
//! - Values the same once outer blanks are dropped, runs of blanks read as
//!   one and letter case is ignored are **one suggestion** with all their
//!   sources, spelled the best-cased way ("Velo Records" over "VELO
//!   RECORDS"; among equals the strongest source's).
//! - A suggestion equal to the current value (the same way) is dropped, and
//!   so is one the field can't take ("Use" would refuse it).
//! - Ranked by number of sources, then by the strongest source (the linked
//!   file's tags, then a sibling file's, then a file's name, then online),
//!   then by the text: the same order every time.
//!
//! The offline sources are in `offline`. A track with nothing to suggest
//! answers empty lists, not an error, and all of it works with every online
//! service off and no network: nothing here goes online.
//!
//! **Online candidates (job S2)** come in through [`suggestions_with`]: the
//! caller gives its own [`Candidate`]s (with [`IdentifySource::Online`]),
//! and they are merged and ranked with the offline ones by the same rules.
//! This module makes no request and never names a service itself.
//!
//! # Use ([`use_on`], command [`identify_use`])
//!
//! One operation ([`USE_OPERATION`], one Undo) that sets each chosen field
//! as the app's own value, by the edit store's rules (`app_values::set_in`,
//! the path `genre::own` takes: a value equal to rekordbox's current one
//! leaves the field following rekordbox, a first edit records what it
//! replaced), and the catalogue number in its table. The values reach
//! rekordbox on the next send (1.10: only the app changed), and a later
//! change made in rekordbox is taken in or is a conflict, as for any field.
//! Audio files are never written. It refuses, changing nothing: a field not
//! listed above, a blank value (clearing a field isn't Use), a value its
//! field can't take or rekordbox's XML can't carry, and a track that isn't a
//! Library track.

mod choose;
mod offline;
mod rank;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs::JobQueue;
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::send_values::{send_values, NamedFile, Outcome};

pub use choose::{use_on, UseChoice, UseError, USE_OPERATION};

/// A field identification suggests and "Use" sets, by its rekordbox
/// attribute name; the catalogue number has none (it is never sent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum IdentifyField {
    Name,
    Artist,
    Mix,
    Remixer,
    Label,
    Year,
    Album,
    CatalogueNumber,
}

impl IdentifyField {
    /// The fields sent to rekordbox, in the order they are listed.
    pub const SENT: [IdentifyField; 7] = [
        IdentifyField::Name,
        IdentifyField::Artist,
        IdentifyField::Mix,
        IdentifyField::Remixer,
        IdentifyField::Label,
        IdentifyField::Year,
        IdentifyField::Album,
    ];

    /// The rekordbox `TRACK` attribute, `None` for the catalogue number.
    pub fn attribute(self) -> Option<&'static str> {
        Some(match self {
            IdentifyField::Name => "Name",
            IdentifyField::Artist => "Artist",
            IdentifyField::Mix => "Mix",
            IdentifyField::Remixer => "Remixer",
            IdentifyField::Label => "Label",
            IdentifyField::Year => "Year",
            IdentifyField::Album => "Album",
            IdentifyField::CatalogueNumber => return None,
        })
    }

    /// The sent field of a rekordbox attribute name.
    pub fn of_attribute(attribute: &str) -> Option<IdentifyField> {
        IdentifyField::SENT
            .into_iter()
            .find(|f| f.attribute() == Some(attribute))
    }

    /// The field of its name as it crosses to the frontend ("Name", …,
    /// "CatalogueNumber").
    pub fn of_name(name: &str) -> Option<IdentifyField> {
        if name == "CatalogueNumber" {
            Some(IdentifyField::CatalogueNumber)
        } else {
            IdentifyField::of_attribute(name)
        }
    }
}

/// Where a suggested value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum IdentifySource {
    /// The tags of the track's linked file.
    Tags { file: NamedFile },
    /// A file's name, read as "Artist - Title (Mix)".
    FileName { file: NamedFile },
    /// The tags of another file of the same track.
    Sibling { file: NamedFile },
    /// An online service (job S2): `service` is its name, `via` the service
    /// that led to it ("MusicBrainz via AcoustID"). Nothing here gives one.
    Online {
        service: String,
        via: Option<String>,
    },
}

impl IdentifySource {
    /// How much a value from here counts when two suggestions have as many
    /// sources: lower is stronger.
    pub fn strength(&self) -> u8 {
        match self {
            IdentifySource::Tags { .. } => 0,
            IdentifySource::Sibling { .. } => 1,
            IdentifySource::FileName { .. } => 2,
            IdentifySource::Online { .. } => 3,
        }
    }

    /// The order sources are listed in: strongest first, then by file or
    /// service.
    fn order(&self) -> (u8, i64, &str, &str) {
        let strength = self.strength();
        match self {
            IdentifySource::Tags { file }
            | IdentifySource::FileName { file }
            | IdentifySource::Sibling { file } => (strength, file.id, "", ""),
            IdentifySource::Online { service, via } => {
                (strength, 0, service, via.as_deref().unwrap_or(""))
            }
        }
    }
}

/// One value a source gave for a field: what [`suggestions_with`] merges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub field: IdentifyField,
    pub value: String,
    pub source: IdentifySource,
}

/// One suggested value with every source that gave it, strongest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentifySuggestion {
    pub value: String,
    pub sources: Vec<IdentifySource>,
}

/// One field: what the next send carries now and the ranked suggestions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentifyFieldSuggestions {
    pub field: IdentifyField,
    /// `None` when the field has no value. For the catalogue number, the
    /// one the app keeps.
    pub current: Option<String>,
    pub suggestions: Vec<IdentifySuggestion>,
}

/// What identification suggests for one Library track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentifySuggestions {
    /// The sent fields, in [`IdentifyField::SENT`] order.
    pub fields: Vec<IdentifyFieldSuggestions>,
    pub catalogue_number: IdentifyFieldSuggestions,
}

/// The current value of each field of `track` (module docs), blanks and a
/// `Year` of 0 as none.
fn currents(
    conn: &Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
) -> rusqlite::Result<HashMap<IdentifyField, String>> {
    let mut out = HashMap::new();
    if let Some(Outcome::Ready(values)) = send_values(conn, volumes, &[track])?
        .pop()
        .map(|s| s.outcome)
    {
        for field in IdentifyField::SENT {
            if let Some(v) = field.attribute().and_then(|a| values.get(a)) {
                out.insert(field, v.value.clone());
            }
        }
    }
    let mut stmt =
        conn.prepare_cached("SELECT field, value FROM app_value WHERE library_track_id = ?1")?;
    let own = stmt
        .query_map([track.0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (attribute, value) in own {
        if let Some(field) = IdentifyField::of_attribute(&attribute) {
            out.insert(field, value);
        }
    }
    if let Some(number) = catalogue_number(conn, track)? {
        out.insert(IdentifyField::CatalogueNumber, number);
    }
    out.retain(|field, v| !v.trim().is_empty() && !(*field == IdentifyField::Year && v == "0"));
    Ok(out)
}

/// The catalogue number the app keeps for `track`, if any.
pub fn catalogue_number(conn: &Connection, track: LibraryTrackId) -> rusqlite::Result<Option<String>> {
    conn.prepare_cached("SELECT value FROM catalogue_number WHERE library_track_id = ?1")?
        .query_row([track.0], |r| r.get(0))
        .optional()
}

/// The suggestions for the Library track `track` from the offline sources;
/// `None` when there's no such Library track.
pub fn suggestions(
    conn: &Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
) -> rusqlite::Result<Option<IdentifySuggestions>> {
    suggestions_with(conn, volumes, track, Vec::new())
}

/// [`suggestions`], with `extra` candidates merged and ranked in by the same
/// rules: the seam online sources (job S2) come in through. A candidate's
/// value is checked as an offline one is (dropped if its field can't take
/// it), so a caller need not.
pub fn suggestions_with(
    conn: &Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
    extra: Vec<Candidate>,
) -> rusqlite::Result<Option<IdentifySuggestions>> {
    let Some(mut all) = offline::candidates(conn, track)? else {
        return Ok(None);
    };
    all.extend(extra);
    let currents = currents(conn, volumes, track)?;
    let mut of_field = |field: IdentifyField| {
        let current = currents.get(&field).cloned();
        let mine: Vec<Candidate> = all.iter().filter(|c| c.field == field).cloned().collect();
        IdentifyFieldSuggestions {
            field,
            suggestions: rank::rank(field, current.as_deref(), mine),
            current,
        }
    };
    let fields = IdentifyField::SENT.into_iter().map(&mut of_field).collect();
    let catalogue_number = of_field(IdentifyField::CatalogueNumber);
    Ok(Some(IdentifySuggestions {
        fields,
        catalogue_number,
    }))
}

/// What identification suggests for a Library track's details, from its own
/// files (ROADMAP 2.2). Works with every online service off.
#[tauri::command]
#[specta::specta]
pub async fn identify_suggestions(
    reads: State<'_, ReadPool>,
    library_track_id: LibraryTrackId,
) -> Result<IdentifySuggestions, IpcError> {
    let volumes = crate::scan::system_volumes();
    reads
        .read(|conn| suggestions(conn, &volumes, library_track_id))?
        .ok_or_else(|| IpcError::new(ErrorKind::LibraryTrackNotFound))
}

/// The most tracks due that "Use" works out before it returns, as for a
/// genre set with Use (`genre::own`).
const SETTLE_NOW_LIMIT: usize = 10;

/// "Use" on suggestions: sets each chosen field of a Library track as the
/// app's own value (the catalogue number in its own table), as one
/// operation, one Undo. Answers the operation recorded, none if nothing
/// changed.
#[tauri::command(async)]
#[specta::specta]
pub fn identify_use(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    library_track_id: LibraryTrackId,
    choices: Vec<UseChoice>,
) -> Result<Option<i64>, IpcError> {
    let volumes = crate::scan::system_volumes();
    let operation = writer
        .call(move |conn| Ok(use_on(conn, &volumes, library_track_id, &choices)))??;
    if operation.is_some() {
        crate::library::settle_now_up_to(&writer, &jobs, SETTLE_NOW_LIMIT);
    }
    Ok(operation)
}
