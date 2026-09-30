//! The app's SQLite database (ROADMAP §0.1).
//!
//! All writes go through one connection owned by one thread ([`Writer`]).
//! Reads go through a pool of read-only connections ([`ReadPool`]). The
//! database is in WAL mode, so reads never wait for the writer. There is no
//! global `Mutex<Connection>`.
//!
//! The writer applies pending [`migrations`] when it opens the database.
//! Open the writer first, then the read pool.

pub mod migrations;
mod read_pool;
mod writer;

use std::path::{Path, PathBuf};

pub use read_pool::ReadPool;
pub(crate) use writer::on_a_writer_thread;
pub use writer::{DbError, Writer};

/// The database file's name inside the app data folder.
pub const DB_FILE_NAME: &str = "tracklist-pro.db";

/// Where the database lives for a given app data folder.
///
/// The data folder is `%APPDATA%\com.tracklistpro.desktop\`, never a synced
/// folder (CLAUDE.md).
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DB_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_is_the_named_file_directly_inside_the_data_folder() {
        let data_dir = Path::new("app-data").join(crate::IDENTIFIER);
        let path = db_path(&data_dir);
        assert_eq!(path.parent(), Some(data_dir.as_path()));
        assert_eq!(path.file_name().unwrap(), DB_FILE_NAME);
    }
}
