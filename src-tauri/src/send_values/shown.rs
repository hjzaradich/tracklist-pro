//! The title and artist the lists show for a track (1aG-9; ROADMAP 1.8).
//!
//! Nothing in Phase 1a fills `recording.title` or `recording.artist`, yet a
//! send already works out each track's `Name` and `Artist`. The lists show
//! those same values, so a track reads the same before and after it is
//! sent. Nothing is stored: the values are worked out when a list is read,
//! from what the scan already stored (no file is opened).
//!
//! - **A Library track** ([`shown`]): exactly what a send would write for
//!   it. The decisions are [`Loaded::source`] and [`Loaded::tag_order`],
//!   the code a send runs: rekordbox's `Name` and `Artist` for a track
//!   rekordbox knows, otherwise the tag pick of [`candidates`] (the linked
//!   file, then the track's other files on disk, best first). A track a
//!   send can't send (its file is missing and rekordbox doesn't know it)
//!   shows the same tag pick, from its stored tags whether or not the files
//!   are on disk now.
//! - **A recording with no Library track** ([`shown_recordings`], All
//!   music): its own tags, over all its files, best first.
//! - **A track removed from the Library** ([`shown_removed`], the manual
//!   removals): rekordbox's own `Name` and `Artist` for the row at the
//!   `Location` it was sent to (what the user looks for in rekordbox), else
//!   its tags as above.
//!
//! In all three, `recording.title` / `recording.artist` win if they're ever
//! set (later phases fill them). With no title the file's name (without the
//! folder) is shown; with no artist, nothing. Blank text says nothing.

use super::*;

/// The title and artist to show for a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Never blank unless the track has no file either.
    pub title: String,
    /// Blank when there is none.
    pub artist: String,
}

impl Shown {
    /// The title and the artist as `Some` text, `None` for blank: the form
    /// the lists carry them in.
    pub fn into_options(self) -> (Option<String>, Option<String>) {
        let some = |text: String| (!text.trim().is_empty()).then_some(text);
        (some(self.title), some(self.artist))
    }
}

/// A value with something in it: blank text says nothing, like none.
fn filled(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !t.trim().is_empty())
}

/// The rule: the track's own title and artist if set, else what a send
/// writes (`sent`), else the file's name and no artist.
fn compose(
    own: (Option<&str>, Option<&str>),
    sent: (Option<&str>, Option<&str>),
    file_name: &str,
) -> Shown {
    Shown {
        title: filled(own.0)
            .or(filled(sent.0))
            .unwrap_or(file_name)
            .to_owned(),
        artist: filled(own.1)
            .or(filled(sent.1))
            .unwrap_or_default()
            .to_owned(),
    }
}

/// An entry's first value of `name`, as `TrackValues::get` finds it.
fn first<'a>(attributes: Option<&'a Vec<(String, String)>>, name: &str) -> Option<&'a str> {
    attributes?
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// The first `Name` and `Artist` among the files' tags, as a send picks them.
fn first_tags(tags: &[(i64, Vec<tag_fields::TagValue>)]) -> (Option<String>, Option<String>) {
    let pick = |attribute: &str| {
        candidates(tags, attribute)
            .first()
            .map(|(_, v)| v.value.clone())
    };
    (pick("Name"), pick("Artist"))
}

const NAMES: [&str; 2] = ["Name", "Artist"];

impl Loaded {
    fn shown(&self, row: &Row) -> Shown {
        let (title, artist) = match self.source(row) {
            Source::Known { entry, .. } => {
                let attributes = self.attributes.get(&entry);
                (
                    first(attributes, "Name").map(str::to_owned),
                    first(attributes, "Artist").map(str::to_owned),
                )
            }
            Source::Files { .. } | Source::Cannot(_) => {
                first_tags(&read_tags(&self.files, &self.tag_order(row), &NAMES))
            }
        };
        // The linked file's name, else the track's best file's.
        let name = row
            .linked_file
            .or_else(|| self.by_recording.get(&row.recording)?.first().copied())
            .and_then(|f| self.files.get(&f))
            .map_or("", |f| f.name.as_str());
        compose(
            (row.title.as_deref(), row.artist.as_deref()),
            (title.as_deref(), artist.as_deref()),
            name,
        )
    }
}

/// The title and artist to show for each Library track in `ids`, in a fixed
/// number of queries however many there are. An id that isn't a Library
/// track has no entry.
pub fn shown(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<HashMap<LibraryTrackId, Shown>> {
    let loaded = load(conn, volumes, ids, Scope::Names)?;
    Ok(ids
        .iter()
        .filter_map(|&id| Some((id, loaded.shown(loaded.rows.get(&id.0)?))))
        .collect())
}

/// What the tags and the name of a recording's files give.
struct Found {
    own_title: Option<String>,
    own_artist: Option<String>,
    title: Option<String>,
    artist: Option<String>,
    /// The first file's name, best file first.
    file_name: String,
}

/// For each recording in `ids`: what its files give (it may have none), in
/// one query. The tags are read file by file only until both are found.
fn found(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, Found>> {
    let wanted = json_ids(&mut ids.iter().copied());
    let mut stmt = conn.prepare(
        "SELECT r.id, r.title, r.artist, f.rel_path, f.raw_tags
         FROM recording r
         LEFT JOIN recording_file rf ON rf.recording_id = r.id
         LEFT JOIN file f ON f.id = rf.file_id
         WHERE r.id IN (SELECT value FROM json_each(?1))
         ORDER BY r.id, (rf.role = 'best') DESC, f.id",
    )?;
    let mut rows = stmt.query([&wanted])?;
    let mut out: HashMap<i64, Found> = HashMap::with_capacity(ids.len());
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let found = match out.entry(id) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let rel_path: Option<String> = row.get(3)?;
                e.insert(Found {
                    own_title: row.get(1)?,
                    own_artist: row.get(2)?,
                    title: None,
                    artist: None,
                    file_name: rel_path
                        .as_deref()
                        .map(file_name)
                        .unwrap_or_default()
                        .to_owned(),
                })
            }
        };
        if found.title.is_some() && found.artist.is_some() {
            continue;
        }
        let Some(raw_tags) = row.get::<_, Option<String>>(4)? else {
            continue;
        };
        for value in tag_fields::read_only(&raw_tags, &NAMES) {
            let slot = match value.attribute {
                "Name" => &mut found.title,
                _ => &mut found.artist,
            };
            slot.get_or_insert(value.value);
        }
    }
    Ok(out)
}

/// The title and artist to show for each recording in `ids`, from its own
/// tags (best file first), in one query. For All music's tracks that aren't
/// in the Library.
pub fn shown_recordings(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, Shown>> {
    Ok(found(conn, ids)?
        .into_iter()
        .map(|(id, f)| {
            let shown = compose(
                (f.own_title.as_deref(), f.own_artist.as_deref()),
                (f.title.as_deref(), f.artist.as_deref()),
                &f.file_name,
            );
            (id, shown)
        })
        .collect())
}

/// A track removed from the Library, to name in the manual removals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub recording_id: i64,
    /// The match key of the `Location` it was last sent to, if that reads
    /// back.
    pub location_key: Option<String>,
    /// The name of the file at that `Location`.
    pub path_name: Option<String>,
}

/// The title and artist to show for each removed track, in the order given:
/// rekordbox's own at the `Location` it was sent to (any row the store holds
/// there, the same rows the manual-removals list counts; the lowest
/// `TrackID` if it holds several), else the recording's tags, else the file's
/// name (the sent path's, else the recording's best file's).
pub fn shown_removed(conn: &Connection, removed: &[Removed]) -> rusqlite::Result<Vec<Shown>> {
    let keys = serde_json::to_string(
        &removed
            .iter()
            .filter_map(|r| r.location_key.as_deref())
            .collect::<Vec<_>>(),
    )
    .expect("strings always serialize");
    // The rows the manual-removals list itself counts: any row the store
    // holds at the Location, including one an incomplete read kept from an
    // earlier read. Where several share it the lowest `TrackID` wins
    // (listed last, so it overwrites).
    let mut in_rekordbox: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT location_key, json_extract(attributes, '$.Name'),
                json_extract(attributes, '$.Artist')
         FROM rekordbox_track
         WHERE location_key IN (SELECT value FROM json_each(?1))
         ORDER BY track_id DESC",
    )?;
    for row in stmt.query_map([&keys], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))? {
        let (key, names): (String, (Option<String>, Option<String>)) = row?;
        in_rekordbox.insert(key, names);
    }
    let own = found(
        conn,
        &removed.iter().map(|r| r.recording_id).collect::<Vec<_>>(),
    )?;
    Ok(removed
        .iter()
        .map(|r| {
            let f = own.get(&r.recording_id);
            let rekordbox = r
                .location_key
                .as_ref()
                .and_then(|key| in_rekordbox.get(key));
            let (title, artist) = match (rekordbox, f) {
                (Some((title, artist)), _) => (title.as_deref(), artist.as_deref()),
                (None, Some(f)) => (f.title.as_deref(), f.artist.as_deref()),
                (None, None) => (None, None),
            };
            compose(
                (
                    f.and_then(|f| f.own_title.as_deref()),
                    f.and_then(|f| f.own_artist.as_deref()),
                ),
                (title, artist),
                r.path_name
                    .as_deref()
                    .or(f.map(|f| f.file_name.as_str()))
                    .unwrap_or_default(),
            )
        })
        .collect())
}
