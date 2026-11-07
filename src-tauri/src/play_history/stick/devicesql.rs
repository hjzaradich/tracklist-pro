//! Reads the history of a gig stick's `export.pdb`, rekordbox's DeviceSQL
//! export database, from its bytes. Written from Deep Symmetry's published
//! description of the format (the crate-digger analysis cited in
//! `spikes/pdb_history.py`); no code of theirs is used.
//!
//! Only four tables are read: tracks (for each track's path on the stick,
//! title and artist), artists, history lists ("HISTORY 001", ...) and
//! history entries (track, list, place in the list).
//!
//! Layout, little-endian throughout:
//! - The file is pages of `page_len` bytes. Page 0 holds the header: `u32 0,
//!   u32 page_len, u32 table count, u32, u32, u32, u32`, then per table
//!   (from 0x1c) `u32 type, u32 empty page, u32 first page, u32 last page`.
//! - A table is a chain of pages from its first to its last, each naming the
//!   next. A page's header is 0x28 bytes: `u32 0, u32 index, u32 type, u32
//!   next page, u32, u32, u8 rows (small), u8, u8, u8 flags, u16, u16, u16,
//!   u16 rows (large), u16, u16`. A page with flag 0x40 holds no rows.
//! - Rows live in the page's heap, from 0x28. Their places are kept at the
//!   page's end in groups of 16, the first group last: a group's base is
//!   `page_len - 0x24 * group`; at base - 4 a `u16` of present flags (bit i
//!   for row i), and row i's heap offset is the `u16` at base - 6 - 2i.
//! - Strings ("DeviceSQL strings"): a first byte 0x40 is long ASCII and 0x90
//!   long UTF-16LE, each with a `u16` total length (header of 4 included)
//!   after it; an odd first byte is a short ASCII string of `(byte >> 1) -
//!   1` bytes.
//!
//! **Never trusted:** every offset, length and count is checked against the
//! bytes there are. A page or row that doesn't add up is skipped and
//! counted ([`Export::skipped`]); it never stops the read of the rest.

use std::collections::{HashMap, HashSet};

/// The biggest `export.pdb` read. A stick with tens of thousands of tracks
/// stays well under it.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;

const TRACKS: u32 = 0;
const ARTISTS: u32 = 2;
const HISTORY_LISTS: u32 = 11;
const HISTORY_ENTRIES: u32 = 12;

const HEADER_TABLES: usize = 0x1c;
const TABLE_ENTRY: usize = 16;
const PAGE_HEADER: usize = 0x28;
const ROW_GROUP: usize = 0x24;
const ROWS_PER_GROUP: usize = 16;
/// The large row count's "not in use" value.
const NO_LARGE_COUNT: u16 = 0x1fff;
const PAGE_NO_ROWS: u8 = 0x40;

/// Where a track row's string offsets start, and the strings read.
const TRACK_STRINGS: usize = 0x5e;
const TRACK_STRING_COUNT: usize = 21;
const TRACK_TITLE: usize = 17;
const TRACK_PATH: usize = 20;

/// A play as the stick names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickPlay {
    /// The file's path from the stick's root, e.g. `/Contents/a/b.mp3`.
    pub path: String,
    pub artist: Option<String>,
    pub title: Option<String>,
}

/// One history list of the stick, its plays in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryList {
    pub id: u32,
    pub name: String,
    pub plays: Vec<StickPlay>,
}

/// What a stick's `export.pdb` holds of its history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// By name.
    pub lists: Vec<HistoryList>,
    /// Pages, rows and entries that couldn't be read and were left out.
    pub skipped: u32,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The DeviceSQL string at `at` in `page`. `None` when it doesn't fit or
/// isn't one.
fn string_at(page: &[u8], at: usize) -> Option<String> {
    let kind = *page.get(at)?;
    let text = match kind {
        0x40 | 0x90 => {
            let len = usize::from(u16_at(page, at + 1)?);
            let body = page.get(at.checked_add(4)?..at.checked_add(len)?)?;
            if kind == 0x40 {
                body.iter().map(|&b| char::from(b)).collect()
            } else {
                if body.len() % 2 != 0 {
                    return None;
                }
                let units: Vec<u16> = body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                String::from_utf16(&units).ok()?
            }
        }
        k if k & 1 == 1 && k >> 1 >= 1 => {
            let end = at.checked_add(usize::from(k >> 1))?;
            page.get(at + 1..end)?
                .iter()
                .map(|&b| char::from(b))
                .collect()
        }
        _ => return None,
    };
    Some(text.trim_end_matches('\0').to_owned())
}

/// A string that says something, or `None`.
fn some_text(s: Option<String>) -> Option<String> {
    s.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

struct Reader<'a> {
    data: &'a [u8],
    page_len: usize,
    /// type → (first page, last page)
    tables: HashMap<u32, (u32, u32)>,
    skipped: u32,
}

impl Reader<'_> {
    fn skip(&mut self) {
        self.skipped = self.skipped.saturating_add(1);
    }

    /// Calls `row(page, offset)` for each present row of the table `kind`;
    /// a row it can't read (`None`) is counted.
    fn rows(&mut self, kind: u32, mut row: impl FnMut(&[u8], usize) -> Option<()>) {
        let Some(&(first, last)) = self.tables.get(&kind) else {
            return;
        };
        let mut seen = HashSet::new();
        let mut index = first;
        while seen.insert(index) {
            let Some(page) = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_mul(self.page_len))
                .and_then(|start| self.data.get(start..start.checked_add(self.page_len)?))
            else {
                // A page past the end: the chain can't go on.
                self.skip();
                return;
            };
            let next = u32_at(page, 0x0c).unwrap_or(index);
            if u32_at(page, 0x08) != Some(kind) {
                self.skip();
            } else if page[0x1b] & PAGE_NO_ROWS == 0 {
                self.page_rows(page, &mut row);
            }
            if index == last {
                return;
            }
            index = next;
        }
        // Back to a page already read: a loop in the chain.
        self.skip();
    }

    fn page_rows(&mut self, page: &[u8], row: &mut impl FnMut(&[u8], usize) -> Option<()>) {
        let small = usize::from(page[0x18]);
        let large = u16_at(page, 0x22).unwrap_or(0);
        let count = if usize::from(large) > small && large != NO_LARGE_COUNT {
            usize::from(large)
        } else {
            small
        };
        let groups = count.div_ceil(ROWS_PER_GROUP);
        for group in 0..groups {
            // The group's place must stay clear of the page's header.
            let base = match self.page_len.checked_sub(group * ROW_GROUP) {
                Some(base) if base >= PAGE_HEADER + ROW_GROUP => base,
                _ => {
                    self.skip();
                    return;
                }
            };
            let Some(present) = u16_at(page, base - 4) else {
                self.skip();
                return;
            };
            let in_group = (count - group * ROWS_PER_GROUP).min(ROWS_PER_GROUP);
            for i in 0..in_group {
                if present & (1 << i) == 0 {
                    continue;
                }
                let read = u16_at(page, base - 6 - 2 * i)
                    .map(|ofs| PAGE_HEADER + usize::from(ofs))
                    .filter(|&at| at < page.len())
                    .and_then(|at| row(page, at));
                if read.is_none() {
                    self.skip();
                }
            }
        }
    }
}

struct Track {
    artist: u32,
    title: Option<String>,
    path: String,
}

/// Reads the history of the `export.pdb` in `bytes`. `None` when it isn't
/// one (its header doesn't add up).
pub fn parse(bytes: &[u8]) -> Option<Export> {
    if u32_at(bytes, 0)? != 0 {
        return None;
    }
    let page_len = usize::try_from(u32_at(bytes, 4)?).ok()?;
    if !(PAGE_HEADER + 2 * ROW_GROUP..=0x1_0000).contains(&page_len) || bytes.len() < page_len {
        return None;
    }
    let declared = usize::try_from(u32_at(bytes, 8)?).ok()?;
    let fits = (page_len - HEADER_TABLES) / TABLE_ENTRY;
    let mut reader = Reader {
        data: bytes,
        page_len,
        tables: HashMap::new(),
        skipped: 0,
    };
    if declared > fits {
        reader.skip();
    }
    for t in 0..declared.min(fits) {
        let at = HEADER_TABLES + t * TABLE_ENTRY;
        let (Some(kind), Some(first), Some(last)) = (
            u32_at(bytes, at),
            u32_at(bytes, at + 8),
            u32_at(bytes, at + 12),
        ) else {
            reader.skip();
            continue;
        };
        reader.tables.entry(kind).or_insert((first, last));
    }

    let mut artists: HashMap<u32, String> = HashMap::new();
    reader.rows(ARTISTS, |page, at| {
        let subtype = u16_at(page, at)?;
        let id = u32_at(page, at + 4)?;
        let ofs = if subtype & 0x04 != 0 {
            usize::from(u16_at(page, at + 0x0a)?)
        } else {
            usize::from(*page.get(at + 9)?)
        };
        let name = string_at(page, at.checked_add(ofs)?)?;
        artists.entry(id).or_insert(name);
        Some(())
    });

    let mut tracks: HashMap<u32, Track> = HashMap::new();
    reader.rows(TRACKS, |page, at| {
        let artist = u32_at(page, at + 0x44)?;
        let id = u32_at(page, at + 0x48)?;
        let string = |n: usize| {
            u16_at(page, at + TRACK_STRINGS + 2 * n)
                .and_then(|ofs| string_at(page, at.checked_add(usize::from(ofs))?))
        };
        // The last offset must be there for the row to be whole.
        u16_at(page, at + TRACK_STRINGS + 2 * (TRACK_STRING_COUNT - 1))?;
        let path = some_text(string(TRACK_PATH))?;
        tracks.entry(id).or_insert(Track {
            artist,
            title: some_text(string(TRACK_TITLE)),
            path,
        });
        Some(())
    });

    let mut lists: Vec<(u32, String)> = Vec::new();
    reader.rows(HISTORY_LISTS, |page, at| {
        let id = u32_at(page, at)?;
        let name = some_text(string_at(page, at + 4))?;
        lists.push((id, name));
        Some(())
    });

    // (list, place, row order) → track
    let mut entries: Vec<(u32, u32, usize, u32)> = Vec::new();
    reader.rows(HISTORY_ENTRIES, |page, at| {
        let track = u32_at(page, at)?;
        let list = u32_at(page, at + 4)?;
        let place = u32_at(page, at + 8)?;
        entries.push((list, place, entries.len(), track));
        Some(())
    });
    entries.sort_unstable();

    let mut by_list: HashMap<u32, Vec<StickPlay>> = HashMap::new();
    let known: HashSet<u32> = lists.iter().map(|(id, _)| *id).collect();
    for (list, _, _, track) in entries {
        // An entry of no list, or of a track the stick doesn't describe.
        let Some(track) = tracks.get(&track).filter(|_| known.contains(&list)) else {
            reader.skip();
            continue;
        };
        by_list.entry(list).or_default().push(StickPlay {
            path: track.path.clone(),
            artist: artists
                .get(&track.artist)
                .cloned()
                .and_then(|a| some_text(Some(a))),
            title: track.title.clone(),
        });
    }

    let mut seen = HashSet::new();
    let mut out: Vec<HistoryList> = lists
        .into_iter()
        .filter(|(id, _)| seen.insert(*id))
        .map(|(id, name)| HistoryList {
            id,
            plays: by_list.remove(&id).unwrap_or_default(),
            name,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Some(Export {
        lists: out,
        skipped: reader.skipped,
    })
}
