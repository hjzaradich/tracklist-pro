//! Builds small, valid `export.pdb` files for tests, laid out as
//! [`super::devicesql`] describes. Made-up rows only, never a real stick's.

use std::path::Path;

pub const PAGE: usize = 4096;
pub const TRACKS: u32 = 0;
pub const ARTISTS: u32 = 2;
pub const LISTS: u32 = 11;
pub const ENTRIES: u32 = 12;

/// A DeviceSQL string: short ASCII when it fits, long UTF-16LE when it isn't
/// ASCII, long ASCII otherwise.
pub fn dstring(s: &str) -> Vec<u8> {
    if !s.is_ascii() {
        let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut out = vec![0x90];
        out.extend(u16::try_from(units.len() + 4).unwrap().to_le_bytes());
        out.push(0);
        out.extend(units);
        return out;
    }
    if s.len() < 126 {
        let mut out = vec![u8::try_from(((s.len() + 1) << 1) | 1).unwrap()];
        out.extend(s.as_bytes());
        return out;
    }
    let mut out = vec![0x40];
    out.extend(u16::try_from(s.len() + 4).unwrap().to_le_bytes());
    out.push(0);
    out.extend(s.as_bytes());
    out
}

/// A track row: its id, artist id, title and path on the stick.
pub fn track(id: u32, artist: u32, title: &str, path: &str) -> Vec<u8> {
    const STRINGS: usize = 0x5e;
    let mut row = vec![0_u8; STRINGS + 21 * 2];
    row[0..2].copy_from_slice(&0x24_u16.to_le_bytes());
    row[0x44..0x48].copy_from_slice(&artist.to_le_bytes());
    row[0x48..0x4c].copy_from_slice(&id.to_le_bytes());
    for n in 0..21 {
        let text = match n {
            17 => title,
            19 => path.rsplit('/').next().unwrap_or(path),
            20 => path,
            _ => "",
        };
        let at = u16::try_from(row.len()).unwrap();
        row[STRINGS + 2 * n..STRINGS + 2 * n + 2].copy_from_slice(&at.to_le_bytes());
        row.extend(dstring(text));
    }
    row
}

/// An artist row with the near name offset.
pub fn artist(id: u32, name: &str) -> Vec<u8> {
    let mut row = vec![0x60, 0];
    row.extend(0_u16.to_le_bytes());
    row.extend(id.to_le_bytes());
    row.push(0x03);
    row.push(0x0a);
    row.extend(dstring(name));
    row
}

/// A history list row.
pub fn list(id: u32, name: &str) -> Vec<u8> {
    let mut row = id.to_le_bytes().to_vec();
    row.extend(dstring(name));
    row
}

/// A history entry row.
pub fn entry(track: u32, list: u32, place: u32) -> Vec<u8> {
    [track, list, place]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

/// A file of tables, each one chain of pages.
#[derive(Default)]
pub struct Pdb {
    /// (type, pages of rows)
    tables: Vec<(u32, Vec<Vec<Vec<u8>>>)>,
}

impl Pdb {
    pub fn new() -> Pdb {
        Pdb::default()
    }

    /// Adds a table whose rows fill pages of at most `per_page` rows.
    pub fn table(mut self, kind: u32, rows: Vec<Vec<u8>>, per_page: usize) -> Pdb {
        let pages = rows.chunks(per_page.max(1)).map(<[_]>::to_vec).collect();
        self.tables.push((kind, pages));
        self
    }

    /// The page indexes of table `kind`, in chain order.
    pub fn pages_of(&self, kind: u32) -> Vec<u32> {
        let mut next = 1_u32;
        for (k, pages) in &self.tables {
            let n = u32::try_from(pages.len().max(1)).unwrap();
            if *k == kind {
                return (next..next + n).collect();
            }
            next += n;
        }
        Vec::new()
    }

    pub fn build(&self) -> Vec<u8> {
        let mut file = vec![0_u8; PAGE];
        let put =
            |buf: &mut [u8], at: usize, v: u32| buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut file, 4, PAGE as u32);
        put(&mut file, 8, u32::try_from(self.tables.len()).unwrap());
        for (t, (kind, pages)) in self.tables.iter().enumerate() {
            let indexes = self.pages_of(*kind);
            let at = 0x1c + 16 * t;
            put(&mut file, at, *kind);
            put(&mut file, at + 4, 0);
            put(&mut file, at + 8, indexes[0]);
            put(&mut file, at + 12, *indexes.last().unwrap());
            let empty = vec![Vec::new()];
            let pages = if pages.is_empty() { &empty } else { pages };
            for (p, rows) in pages.iter().enumerate() {
                let index = indexes[p];
                let next = indexes.get(p + 1).copied().unwrap_or(index + 1000);
                let mut page = vec![0_u8; PAGE];
                put(&mut page, 4, index);
                put(&mut page, 8, *kind);
                put(&mut page, 12, next);
                page[0x18] = u8::try_from(rows.len()).unwrap();
                page[0x1b] = 0x24;
                page[0x22..0x24].copy_from_slice(&u16::try_from(rows.len()).unwrap().to_le_bytes());
                let mut heap = 0x28;
                for (r, row) in rows.iter().enumerate() {
                    page[heap..heap + row.len()].copy_from_slice(row);
                    let base = PAGE - (r / 16) * 0x24;
                    let i = r % 16;
                    let ofs = u16::try_from(heap - 0x28).unwrap();
                    page[base - 6 - 2 * i..base - 4 - 2 * i].copy_from_slice(&ofs.to_le_bytes());
                    let flags = u16::from_le_bytes([page[base - 4], page[base - 3]]) | (1 << i);
                    page[base - 4..base - 2].copy_from_slice(&flags.to_le_bytes());
                    heap += row.len();
                    // Rows stay 4-byte aligned, as rekordbox writes them.
                    heap = heap.div_ceil(4) * 4;
                }
                file.extend(page);
            }
        }
        file
    }
}

/// A small stick's history: two lists, made-up tracks; the second list's
/// entries are stored out of order.
pub fn two_gigs() -> Pdb {
    Pdb::new()
        .table(
            ARTISTS,
            vec![artist(1, "Made Up Artist"), artist(2, "Другой Артист")],
            16,
        )
        .table(
            TRACKS,
            vec![
                track(10, 1, "First Tune", "/Contents/Made Up Artist/first.mp3"),
                track(11, 2, "Second Tune", "/Contents/Другой/second.flac"),
                track(12, 9, "Third Tune", "/Music/a.mp3"),
            ],
            2,
        )
        .table(
            LISTS,
            vec![list(2, "HISTORY 002"), list(1, "HISTORY 001")],
            16,
        )
        .table(
            ENTRIES,
            vec![
                entry(10, 1, 1),
                entry(11, 1, 2),
                entry(12, 2, 3),
                entry(10, 2, 1),
                entry(11, 2, 2),
            ],
            16,
        )
}

/// Writes `bytes` as the export database of a stick rooted at `root`.
pub fn write_stick(root: &Path, bytes: &[u8]) {
    // ci-reads: not a frontend read
    let dir = root.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("export.pdb"), bytes).unwrap();
}
