//! Reading a picked bundle safely, and checking a database before it can
//! become the Library. Everything here works on a copy in app data's
//! staging folder; the live database is never opened.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode};

use super::{BundleError, Counts, Manifest, APP, DB_ENTRY, FORMAT, MANIFEST_ENTRY};
use crate::db::migrations::{self, MIGRATIONS};
use crate::write_guard::{GuardedPath, Staging};

/// The most a bundle's entries may unpack to (a zip bomb is refused before
/// it fills the disk).
pub(super) struct Caps {
    pub db: u64,
    pub manifest: u64,
}

pub(super) const CAPS: Caps = Caps {
    db: 16 << 30,
    manifest: 64 << 10,
};

/// Unpacks the bundle at `zip` into `staging`: exactly the two expected
/// entries, each within its cap. Returns the manifest and the database's
/// staged path.
pub(super) fn unpack(
    zip: &Path,
    staging: &Staging,
    caps: &Caps,
) -> Result<(Manifest, PathBuf), BundleError> {
    let file = File::open(zip).map_err(|_| BundleError::NotOurs)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| BundleError::NotOurs)?;
    // Exactly our two names: nothing extra, no folder, no path, no `..`.
    let mut names = BTreeSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|_| BundleError::NotOurs)?;
        // The raw bytes, compared exactly: no decoding to trust.
        let cap = match entry.name_raw() {
            name if name == MANIFEST_ENTRY.as_bytes() => caps.manifest,
            name if name == DB_ENTRY.as_bytes() => caps.db,
            _ => return Err(BundleError::NotOurs),
        };
        if entry.is_dir() || !names.insert(entry.name_raw().to_vec()) {
            return Err(BundleError::NotOurs);
        }
        if entry.size() > cap {
            return Err(BundleError::TooLarge);
        }
    }
    if names.len() != 2 {
        return Err(BundleError::NotOurs);
    }

    let mut text = Vec::new();
    let entry = archive.by_name(MANIFEST_ENTRY).map_err(|_| BundleError::NotOurs)?;
    entry
        .take(caps.manifest + 1)
        .read_to_end(&mut text)
        .map_err(|_| BundleError::Damaged)?;
    if text.len() as u64 > caps.manifest {
        return Err(BundleError::TooLarge);
    }
    let manifest: Manifest = serde_json::from_slice(&text).map_err(|_| BundleError::NotOurs)?;
    if manifest.app != APP {
        return Err(BundleError::NotOurs);
    }
    if manifest.format > FORMAT {
        return Err(BundleError::Newer);
    }

    // The header's size can lie: the stream itself is cut at the cap.
    let entry = archive.by_name(DB_ENTRY).map_err(|_| BundleError::NotOurs)?;
    let mut reader = Capped {
        inner: entry.take(caps.db + 1),
        read: 0,
        failed: false,
    };
    let copied = staging.copy_in(DB_ENTRY, &mut reader);
    if reader.failed {
        // A bad deflate stream or checksum.
        return Err(BundleError::Damaged);
    }
    let path = copied?;
    if reader.read > caps.db {
        return Err(BundleError::TooLarge);
    }
    Ok((manifest, path))
}

/// A reader that counts what it reads and remembers a failure, so a
/// damaged entry isn't reported as a failed write.
struct Capped<R> {
    inner: R,
    read: u64,
    failed: bool,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf).inspect_err(|_| self.failed = true)?;
        self.read += n as u64;
        Ok(n)
    }
}

/// What a checked database holds.
pub(super) struct Checked {
    pub schema_version: u32,
    pub counts: Counts,
}

/// Checks the staged database at `path`: SQLite, undamaged, its foreign
/// keys whole, and our schema at this build's version or an older one.
/// With `migrate`, an older one is brought up to date (on this copy), and
/// the file is left as one file (no `-wal`) ready to be moved.
pub(super) fn database(path: &GuardedPath, migrate: bool) -> Result<Checked, BundleError> {
    let mut header = [0u8; 16];
    File::open(path.as_path())
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|_| BundleError::NotOurs)?;
    if &header != b"SQLite format 3\0" {
        return Err(BundleError::NotOurs);
    }
    let mut conn = path.open_database().map_err(damaged)?;
    let verdict: Vec<String> = conn
        .prepare("PRAGMA integrity_check")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(damaged)?;
    if verdict != ["ok"] {
        return Err(BundleError::Damaged);
    }
    let broken: bool = conn
        .query_row("SELECT EXISTS (SELECT 1 FROM pragma_foreign_key_check)", [], |r| r.get(0))
        .map_err(damaged)?;
    if broken {
        return Err(BundleError::Damaged);
    }

    let applied = migrations::applied(&conn).map_err(damaged)?;
    let ours_up_to = MIGRATIONS.last().map_or(0, |m| m.version());
    if applied.is_empty() {
        return Err(BundleError::NotOurs);
    }
    for done in &applied {
        match MIGRATIONS.iter().find(|m| m.version() == done.version) {
            None if done.version > ours_up_to => return Err(BundleError::Newer),
            Some(m) if m.name == done.name && m.checksum() == done.checksum => {}
            _ => return Err(BundleError::NotOurs),
        }
    }
    if migrate {
        migrations::run(&mut conn, MIGRATIONS)?;
        let broken: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_foreign_key_check)",
            [],
            |r| r.get(0),
        )?;
        if broken {
            return Err(BundleError::Damaged);
        }
        conn.pragma_update(None, "journal_mode", "DELETE")?;
    }
    let schema_version = migrations::applied(&conn)?
        .last()
        .map_or(0, |a| a.version);
    Ok(Checked {
        schema_version,
        counts: counts(&conn)?,
    })
}

/// The Library's size in a few numbers, for a confirm screen.
pub(super) fn counts(conn: &Connection) -> rusqlite::Result<Counts> {
    conn.query_row(
        "SELECT (SELECT count(*) FROM library_track), (SELECT count(*) FROM crate),
                (SELECT count(*) FROM playlist), (SELECT count(*) FROM tag),
                (SELECT count(*) FROM music_folder)",
        [],
        |r| {
            Ok(Counts {
                tracks: r.get(0)?,
                crates: r.get(1)?,
                playlists: r.get(2)?,
                tags: r.get(3)?,
                music_folders: r.get(4)?,
            })
        },
    )
}

/// A file SQLite can't read as a database is a damaged bundle; anything
/// else is a database failure.
fn damaged(e: rusqlite::Error) -> BundleError {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => BundleError::Damaged,
        _ => BundleError::Db(e.into()),
    }
}
