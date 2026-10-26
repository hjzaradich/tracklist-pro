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

/// The first value of each of `wanted` among the files' tags, as a send
/// picks them.
fn picked_tags(tags: &[(i64, Vec<tag_fields::TagValue>)], wanted: &[&str]) -> Vec<Option<String>> {
    wanted
        .iter()
        .map(|attribute| {
            candidates(tags, attribute)
                .first()
                .map(|(_, v)| v.value.clone())
        })
        .collect()
}

const NAMES: [&str; 2] = ["Name", "Artist"];

/// What the search index and the track table hold of a track besides its
/// files' names: the attributes a list's title and artist come from, five
/// more for the search, and two for the table (rekordbox's length and
/// rating; no tag gives either, so a track rekordbox doesn't know has
/// none). Title and artist come first, so `SEARCH_ATTRIBUTES[..2] == NAMES`.
pub const SEARCH_ATTRIBUTES: [&str; 9] = [
    "Name",
    "Artist",
    "Album",
    "Remixer",
    "Label",
    "Genre",
    "Comments",
    "TotalTime",
    "Rating",
];

impl Loaded {
    /// The first value of each of `wanted` from where the track's values
    /// come from: rekordbox's entry for a track it knows, otherwise the
    /// tag pick of its files ([`Loaded::tag_order`]).
    fn picked(&self, row: &Row, wanted: &[&str]) -> Vec<Option<String>> {
        match self.source(row) {
            Source::Known { entry, .. } => {
                let attributes = self.attributes.get(&entry);
                wanted
                    .iter()
                    .map(|name| first(attributes, name).map(str::to_owned))
                    .collect()
            }
            Source::Files { .. } | Source::Cannot(_) => picked_tags(
                &read_tags(&self.files, &self.tag_order(row), wanted),
                wanted,
            ),
        }
    }

    /// The name of the track's linked file, else its best file's.
    fn file_name_of(&self, row: &Row) -> &str {
        row.linked_file
            .or_else(|| self.by_recording.get(&row.recording)?.first().copied())
            .and_then(|f| self.files.get(&f))
            .map_or("", |f| f.name.as_str())
    }

    fn shown(&self, row: &Row) -> Shown {
        self.shown_with(row, true)
    }

    /// [`Loaded::shown`], with or without the file's name standing in for
    /// a missing title.
    fn shown_with(&self, row: &Row, name_fallback: bool) -> Shown {
        let picked = self.picked(row, &NAMES);
        compose(
            (row.title.as_deref(), row.artist.as_deref()),
            (picked[0].as_deref(), picked[1].as_deref()),
            if name_fallback {
                self.file_name_of(row)
            } else {
                ""
            },
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
    let loaded = load(conn, volumes, ids, Scope::Names, &HashMap::new())?;
    Ok(ids
        .iter()
        .filter_map(|&id| Some((id, loaded.shown(loaded.rows.get(&id.0)?))))
        .collect())
}

/// [`shown`], except that a track with no title of its own, in rekordbox or
/// in its tags has a blank title: the file's name doesn't stand in for it.
/// For counting tracks whose tags are missing.
pub fn shown_tags(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<HashMap<LibraryTrackId, Shown>> {
    let loaded = load(conn, volumes, ids, Scope::Names, &HashMap::new())?;
    Ok(ids
        .iter()
        .filter_map(|&id| Some((id, loaded.shown_with(loaded.rows.get(&id.0)?, false))))
        .collect())
}

/// What the tags and the name of a recording's files give.
struct Found {
    own_title: Option<String>,
    own_artist: Option<String>,
    /// The first value of each attribute asked for among the files' tags,
    /// best file first (`Name` and `Artist` are the first two).
    picked: Vec<Option<String>>,
    /// The first file's name, best file first.
    file_name: String,
}

/// For each recording in `ids`: what its files give (it may have none), in
/// one query. The tags are read file by file only until both are found.
fn found(conn: &Connection, ids: &[i64], wanted: &[&str]) -> rusqlite::Result<HashMap<i64, Found>> {
    let json = json_ids(&mut ids.iter().copied());
    let mut stmt = conn.prepare(
        "SELECT r.id, r.title, r.artist, f.rel_path, f.raw_tags
         FROM recording r
         LEFT JOIN recording_file rf ON rf.recording_id = r.id
         LEFT JOIN file f ON f.id = rf.file_id
         WHERE r.id IN (SELECT value FROM json_each(?1))
         ORDER BY r.id, (rf.role = 'best') DESC, f.id",
    )?;
    let mut rows = stmt.query([&json])?;
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
                    picked: vec![None; wanted.len()],
                    file_name: rel_path
                        .as_deref()
                        .map(file_name)
                        .unwrap_or_default()
                        .to_owned(),
                })
            }
        };
        if found.picked.iter().all(Option::is_some) {
            continue;
        }
        let Some(raw_tags) = row.get::<_, Option<String>>(4)? else {
            continue;
        };
        for value in tag_fields::read_only(&raw_tags, wanted) {
            if let Some(at) = wanted.iter().position(|w| *w == value.attribute) {
                found.picked[at].get_or_insert(value.value);
            }
        }
    }
    Ok(out)
}

/// A rekordbox `Rating` as stars: one of its six values, or none.
fn stars_of(rating: &str) -> Option<u8> {
    match rating.trim() {
        "0" => Some(0),
        "51" => Some(1),
        "102" => Some(2),
        "153" => Some(3),
        "204" => Some(4),
        "255" => Some(5),
        _ => None,
    }
}

/// The title and artist to show for each recording in `ids`, from its own
/// tags (best file first), in one query. For All music's tracks that aren't
/// in the Library.
pub fn shown_recordings(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, Shown>> {
    Ok(found(conn, ids, &NAMES)?
        .into_iter()
        .map(|(id, f)| {
            let shown = compose(
                (f.own_title.as_deref(), f.own_artist.as_deref()),
                (f.picked[0].as_deref(), f.picked[1].as_deref()),
                &f.file_name,
            );
            (id, shown)
        })
        .collect())
}

/// The text of a track the search index holds, besides its files' names:
/// the title and artist the lists show, and the album, remixer, label, genre
/// and comment from the same source as they come from (rekordbox's entry for
/// a Library track it knows, otherwise the best file's tags that have the
/// field). Blank where there is none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShownFields {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub remixer: String,
    pub label: String,
    pub genre: String,
    pub comment: String,
    /// rekordbox's `TotalTime` in seconds, for a Library track it knows.
    pub total_time_s: Option<u32>,
    /// rekordbox's `Rating` in stars (0 to 5), for a Library track it knows
    /// whose rating is one of its six values.
    pub stars: Option<u8>,
    /// The title is the file's name, because the track has none of its own:
    /// the rule's last resort ([`compose`]), known from where the title
    /// came from, never from how it looks.
    pub title_is_file_name: bool,
}

impl ShownFields {
    fn new(shown: Shown, own: Option<&str>, picked: &[Option<String>]) -> ShownFields {
        let at = |i: usize| picked[i].clone().unwrap_or_default();
        ShownFields {
            title_is_file_name: filled(own).or(filled(picked[0].as_deref())).is_none(),
            title: shown.title,
            artist: shown.artist,
            album: at(2),
            remixer: at(3),
            label: at(4),
            genre: at(5),
            comment: at(6),
            total_time_s: picked[7].as_deref().and_then(|t| t.trim().parse().ok()),
            stars: picked[8].as_deref().and_then(stars_of),
        }
    }
}

/// [`shown`] with the other fields the search index holds, for each Library
/// track in `ids`. The title and artist are exactly [`shown`]'s.
pub fn shown_fields(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<HashMap<LibraryTrackId, ShownFields>> {
    let loaded = load(conn, volumes, ids, Scope::Search, &HashMap::new())?;
    Ok(ids
        .iter()
        .filter_map(|&id| {
            let row = loaded.rows.get(&id.0)?;
            let picked = loaded.picked(row, &SEARCH_ATTRIBUTES);
            let shown = compose(
                (row.title.as_deref(), row.artist.as_deref()),
                (picked[0].as_deref(), picked[1].as_deref()),
                loaded.file_name_of(row),
            );
            Some((id, ShownFields::new(shown, row.title.as_deref(), &picked)))
        })
        .collect())
}

/// [`shown_recordings`] with the other fields the search index holds, for
/// each recording in `ids`: its own files' tags, best file first.
pub fn shown_recording_fields(
    conn: &Connection,
    ids: &[i64],
) -> rusqlite::Result<HashMap<i64, ShownFields>> {
    Ok(found(conn, ids, &SEARCH_ATTRIBUTES)?
        .into_iter()
        .map(|(id, f)| {
            let shown = compose(
                (f.own_title.as_deref(), f.own_artist.as_deref()),
                (f.picked[0].as_deref(), f.picked[1].as_deref()),
                &f.file_name,
            );
            (
                id,
                ShownFields::new(shown, f.own_title.as_deref(), &f.picked),
            )
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
        &NAMES,
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
                (None, Some(f)) => (f.picked[0].as_deref(), f.picked[1].as_deref()),
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
