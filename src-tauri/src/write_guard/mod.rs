//! The write guard (ROADMAP §5.6: read-only is enforced in code).
//!
//! The app reads files anywhere but writes only inside folders it owns.
//! Today that's the app data folder, `%APPDATA%\com.tracklistpro.desktop\`;
//! the Library folder joins it as a second root in Phase 2 (ROADMAP 2.6).
//!
//! [`WriteGuard`] is the only way app code gets a writable file handle,
//! creates, renames or deletes a file or folder, or picks where SQLite
//! writes. Everything else in the app only reads, and a source scan in the
//! tests fails if code outside this module calls a writing API.
//!
//! Every path is checked before it's used:
//! 1. made absolute with `.` and `..` resolved, and on Windows turned into
//!    its `\\?\` form so no name is rewritten (see [`crate::paths`]);
//! 2. its deepest existing part is resolved on disk, following symlinks and
//!    junctions, and the rest (which doesn't exist yet) is added back;
//! 3. the result must be one of the roots or inside it.
//!
//! The write then goes to that checked path, never to the caller's
//! spelling, so what was checked is what gets opened. A link inside a root
//! that points outside is refused, and so is a link that points nowhere.
//!
//! A hard link is a second name for the same file, and its other name can
//! be anywhere on the volume, so a path inside a root can't show that the
//! file is. Every file the guard opens for writing is checked on the open
//! handle, before a byte changes (nothing is emptied at open time), and
//! refused if it has more than one name. Renaming or deleting a name
//! doesn't touch the file's contents, so those stay allowed.
//!
//! SQLite is opened here too, read-write or read-only, and each connection
//! refuses `ATTACH` of any file and `VACUUM INTO`: both would make SQLite
//! create or open a database file elsewhere. Even a read-only connection
//! creates `-wal` and `-shm` files beside a WAL database, so every
//! connection, reader or writer, is opened from a [`GuardedPath`].

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{ffi, Connection, OpenFlags};

#[cfg(test)]
mod no_writes_elsewhere;
#[cfg(test)]
mod scan;
#[cfg(test)]
mod tests;

/// A folder the app may write in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// `%APPDATA%\com.tracklistpro.desktop\`: the database and app state.
    AppData,
    // The Library folder is added in Phase 2 (ROADMAP 2.6).
}

/// Hands out every write the app makes, and only inside its roots. Cheap to
/// clone; the app keeps one in Tauri's state (`State<WriteGuard>`).
#[derive(Debug, Clone)]
pub struct WriteGuard {
    /// Each root as it really is on disk: links resolved, `\\?\` form on
    /// Windows.
    roots: Vec<(RootKind, PathBuf)>,
}

/// A path the guard has checked: resolved on disk and inside one of its
/// roots. Only the guard can make one, so code holding one can write there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedPath {
    path: PathBuf,
    root: RootKind,
}

/// Why the guard refused a path or a write failed.
#[derive(Debug)]
pub enum GuardError {
    /// After resolving `..` and links, the path isn't inside any root.
    Outside(PathBuf),
    /// Not a full path from a drive or share root (e.g. `data\x.db`).
    NotAbsolute(PathBuf),
    /// A root must be a folder, not a whole drive.
    RootTooBroad(PathBuf),
    /// A root itself (or a folder holding one) can't be renamed or deleted.
    IsRoot(PathBuf),
    /// The file has more than one name (hard links); another name may be
    /// outside every root, so writing to it could change a file out there.
    HardLinked(PathBuf),
    /// A database file SQLite would open is a link or a folder, not a
    /// plain file.
    NotAPlainFile(PathBuf),
    /// The path couldn't be resolved on disk (e.g. a link to nowhere, or
    /// a drive that isn't there), so it can't be shown to be inside a root.
    Unresolvable { path: PathBuf, source: io::Error },
    /// The check passed but the write itself failed.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardError::Outside(p) => {
                write!(
                    f,
                    "{} is outside the folders the app writes in",
                    p.display()
                )
            }
            GuardError::NotAbsolute(p) => write!(f, "{} isn't a full path", p.display()),
            GuardError::RootTooBroad(p) => {
                write!(f, "{} is a whole drive, not a folder", p.display())
            }
            GuardError::IsRoot(p) => {
                write!(f, "{} holds a folder the app writes in", p.display())
            }
            GuardError::NotAPlainFile(p) => write!(f, "{} isn't a plain file", p.display()),
            GuardError::HardLinked(p) => write!(
                f,
                "{} has other names (hard links), maybe outside the app's folders",
                p.display()
            ),
            GuardError::Unresolvable { path, source } => {
                write!(f, "can't resolve {}: {source}", path.display())
            }
            GuardError::Io { path, source } => {
                write!(f, "can't write {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for GuardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GuardError::Unresolvable { source, .. } | GuardError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl WriteGuard {
    /// Guards the app data folder, creating it (and any missing parents)
    /// first. Creating its own root is the one write the guard makes
    /// without a check.
    pub fn app_data(dir: &Path) -> Result<WriteGuard, GuardError> {
        let dir = lexical(dir)?;
        fs::create_dir_all(&dir).map_err(|source| GuardError::Io {
            path: dir.clone(),
            source,
        })?;
        let root = real_path(&dir)?;
        if !has_a_folder_name(&root) {
            return Err(GuardError::RootTooBroad(root));
        }
        Ok(WriteGuard {
            roots: vec![(RootKind::AppData, root)],
        })
    }

    /// The app data folder, as it really is on disk.
    pub fn app_data_dir(&self) -> &Path {
        self.root(RootKind::AppData)
            .expect("every guard has an app data root")
    }

    /// Where a root is on disk, if this guard has it.
    pub fn root(&self, kind: RootKind) -> Option<&Path> {
        self.roots
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, p)| p.as_path())
    }

    /// Checks that `path` is inside a root once `..` and links are
    /// resolved, and returns the resolved path.
    pub fn check(&self, path: &Path) -> Result<GuardedPath, GuardError> {
        let real = real_path(&lexical(path)?)?;
        self.roots
            .iter()
            .find(|(_, root)| within(&real, root))
            .map(|(kind, _)| GuardedPath {
                path: real.clone(),
                root: *kind,
            })
            .ok_or(GuardError::Outside(real))
    }

    /// Checks the entry at `path` itself, for renaming or deleting it: its
    /// folder is resolved (links followed) but its own name isn't, so a
    /// link there is renamed or deleted, never what it points to. A root,
    /// or a folder holding one, is refused.
    fn check_entry(&self, path: &Path) -> Result<GuardedPath, GuardError> {
        let tidy = lexical(path)?;
        let (Some(parent), Some(name)) = (tidy.parent(), tidy.file_name()) else {
            return Err(GuardError::IsRoot(tidy));
        };
        let real = lexical(&real_path(parent)?.join(name))?;
        let Some((kind, _)) = self.roots.iter().find(|(_, root)| within(&real, root)) else {
            return Err(GuardError::Outside(real));
        };
        if self.roots.iter().any(|(_, root)| within(root, &real)) {
            return Err(GuardError::IsRoot(real));
        }
        Ok(GuardedPath {
            path: real,
            root: *kind,
        })
    }

    /// Creates a folder and any missing parents.
    pub fn create_dir_all(&self, path: &Path) -> Result<GuardedPath, GuardError> {
        let checked = self.check(path)?;
        fs::create_dir_all(&checked.path).map_err(|e| checked.io(e))?;
        Ok(checked)
    }

    /// Creates a file, or empties an existing one, and opens it for writing.
    pub fn create_file(&self, path: &Path) -> Result<File, GuardError> {
        let checked = self.check(path)?;
        let file = checked.open_one_name(true)?;
        file.set_len(0).map_err(|e| checked.io(e))?;
        Ok(file)
    }

    /// Opens an existing file for reading and writing, without emptying it.
    pub fn open_for_writing(&self, path: &Path) -> Result<File, GuardError> {
        self.check(path)?.open_one_name(false)
    }

    /// Writes `bytes` as the whole file, creating it if needed.
    pub fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), GuardError> {
        use std::io::Write;
        let checked = self.check(path)?;
        let mut file = checked.open_one_name(true)?;
        file.set_len(0)
            .and_then(|()| file.write_all(bytes))
            .map_err(|e| checked.io(e))
    }

    /// Writes `bytes` as the whole file at `path` without ever leaving a
    /// half-written file there: the bytes go to a new file beside it, are
    /// flushed to disk, and that file is then renamed over `path`. On any
    /// failure the new file is deleted, and `path` is as it was (missing,
    /// or still holding its old content).
    ///
    /// `path` passes the same checks as [`Self::write`]. The file beside
    /// it gets a name this method picks and is created new, never opened
    /// if something already has that name; it's the only file this ever
    /// deletes.
    ///
    /// The new file is `<name>.part` in `path`'s own folder. If something
    /// already has that name (a file a crashed write left, or anything
    /// else) it's left alone, never overwritten and never deleted, and
    /// `<name>.1.part`, `<name>.2.part`, … are tried, a fixed number of
    /// them; when all are taken the write fails. Clearing leftovers is the
    /// caller's job.
    pub fn write_then_rename(&self, path: &Path, bytes: &[u8]) -> Result<(), GuardError> {
        use std::io::Write;
        self.write_then_rename_with(path, |file| file.write_all(bytes), || Ok(()))
    }

    /// [`Self::write_then_rename`] with its two steps open to tests:
    /// `fill` writes the new file, and `before_rename` runs once it's
    /// flushed. An error from either is a failed write.
    fn write_then_rename_with(
        &self,
        path: &Path,
        fill: impl FnOnce(&mut File) -> io::Result<()>,
        before_rename: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), GuardError> {
        let checked = self.check(path)?;
        // A file already there must have one name, as for any write; it's
        // opened without creating or changing it.
        match checked.open_one_name(false) {
            Ok(_) => {}
            Err(GuardError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let (temp, mut file) = checked.create_beside()?;
        // The flush to disk can't be seen by a test (a file reads back the
        // same without it); it's what makes the rename safe across a crash.
        let done = fill(&mut file)
            .and_then(|()| file.sync_all())
            .and_then(|()| {
                drop(file);
                before_rename()
            })
            .and_then(|()| fs::rename(&temp, &checked.path));
        done.map_err(|e| {
            // The file this call created, and nothing else.
            let _ = fs::remove_file(&temp);
            checked.io(e)
        })
    }

    /// Renames or moves a file, folder or link. Both ends must be inside a
    /// root, and a root itself can't be moved. A link is moved as a link.
    pub fn rename(&self, from: &Path, to: &Path) -> Result<(), GuardError> {
        let from = self.check_entry(from)?;
        let to = self.check_entry(to)?;
        fs::rename(&from.path, &to.path).map_err(|e| from.io(e))
    }

    /// Deletes a file. A link is deleted as a link; what it points to stays.
    pub fn remove_file(&self, path: &Path) -> Result<(), GuardError> {
        let checked = self.check_entry(path)?;
        fs::remove_file(&checked.path).map_err(|e| checked.io(e))
    }

    /// Deletes a folder and everything in it. Links (the folder itself, or
    /// any inside it) are removed, never followed, so nothing they point to
    /// is touched. A root can't be deleted.
    pub fn remove_dir_all(&self, path: &Path) -> Result<(), GuardError> {
        let checked = self.check_entry(path)?;
        fs::remove_dir_all(&checked.path).map_err(|e| checked.io(e))
    }
}

impl GuardedPath {
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Which root the path is in.
    pub fn root(&self) -> RootKind {
        self.root
    }

    /// Opens (or creates) the SQLite database at this path, read-write.
    ///
    /// SQLite puts its `-wal`, `-shm` and journal files beside the database,
    /// so they stay inside the root too. Temp tables, indexes and sorts are
    /// kept in memory (`temp_store = MEMORY`) rather than spilled to the
    /// system temp folder, and `ATTACH` / `VACUUM INTO` are refused.
    pub fn open_database(&self) -> rusqlite::Result<Connection> {
        self.database_files_have_one_name()?;
        confine(Connection::open(&self.path)?)
    }

    /// Opens the SQLite database at this path read-only, confined like
    /// [`Self::open_database`]. It must already exist.
    pub fn open_database_read_only(&self) -> rusqlite::Result<Connection> {
        self.database_files_have_one_name()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        confine(Connection::open_with_flags(&self.path, flags)?)
    }

    /// Refuses a database whose file, or `-wal`, `-shm` or `-journal` file,
    /// is a link (symlink or junction) or has another name, either of which
    /// could lead outside the root. SQLite opens them itself, following
    /// links, so they're checked just before.
    fn database_files_have_one_name(&self) -> rusqlite::Result<()> {
        let refuse = |error: GuardError| {
            Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::SQLITE_CANTOPEN),
                Some(error.to_string()),
            ))
        };
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut name = self.path.clone().into_os_string();
            name.push(suffix);
            let path = PathBuf::from(name);
            // Not following links: a link (or anything but a plain file)
            // is refused before `File::open` below would follow it.
            match fs::symlink_metadata(&path) {
                Ok(meta) if !meta.file_type().is_file() => {
                    return refuse(GuardError::NotAPlainFile(path));
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return refuse(GuardError::Unresolvable { path, source: e }),
            }
            if let Ok(file) = File::open(&path) {
                if link_count(&file).map_or(true, |n| n > 1) {
                    return refuse(GuardError::HardLinked(path));
                }
            }
        }
        Ok(())
    }

    /// Opens the file for reading and writing (creating it if asked, never
    /// emptying it), then refuses it, unchanged, if it has another name.
    fn open_one_name(&self, create: bool) -> Result<File, GuardError> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(create)
            .open(&self.path)
            .map_err(|e| self.io(e))?;
        match link_count(&file) {
            Ok(1) => Ok(file),
            Ok(_) => Err(GuardError::HardLinked(self.path.clone())),
            Err(e) => Err(self.io(e)),
        }
    }

    /// Creates a new, empty file in this path's folder, named after it
    /// (`<name>.part`, then `<name>.1.part`, …), and opens it for writing.
    /// A name something already has is never opened, link or not: the next
    /// one is tried. The folder is this path's own, so the new file is
    /// inside the same root. An error names the file that couldn't be made.
    fn create_beside(&self) -> Result<(PathBuf, File), GuardError> {
        let name = self.path.file_name().unwrap_or_default();
        let mut last = (self.path.clone(), io::ErrorKind::AlreadyExists.into());
        for attempt in 0..BESIDE_ATTEMPTS {
            let mut temp_name = name.to_owned();
            if attempt > 0 {
                temp_name.push(format!(".{attempt}"));
            }
            temp_name.push(".part");
            let temp = self.path.with_file_name(temp_name);
            match File::options().write(true).create_new(true).open(&temp) {
                Ok(file) => return Ok((temp, file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = (temp, e),
                Err(source) => return Err(GuardError::Io { path: temp, source }),
            }
        }
        let (path, source) = last;
        Err(GuardError::Io { path, source })
    }

    fn io(&self, source: io::Error) -> GuardError {
        GuardError::Io {
            path: self.path.clone(),
            source,
        }
    }
}

/// How many names [`GuardedPath::create_beside`] tries before giving up.
const BESIDE_ATTEMPTS: u32 = 16;

/// Keeps a connection's scratch data in memory and refuses `ATTACH` of a
/// file (which `VACUUM INTO` also uses). Plain `VACUUM` attaches an unnamed
/// scratch database, which `temp_store = MEMORY` keeps in memory; that one
/// is allowed. An `ATTACH` whose file name isn't a plain string (e.g.
/// `ATTACH lower('x')` or `ATTACH ?1`) reaches the check without a name and
/// is refused.
fn confine(conn: Connection) -> rusqlite::Result<Connection> {
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    install_sqlite_rule(&conn)?;
    Ok(conn)
}

/// The guard's rule for one SQLite action: [`Authorization::Deny`] for
/// attaching a file and for changing where SQLite keeps temp data (or, on
/// Windows, data: `data_store_directory`), [`Authorization::Allow`] for
/// everything else. Reading those settings is fine.
///
/// A connection has one authorizer. Code that installs its own for a while
/// (the migration runner does) must deny whatever this denies, and put the
/// rule back with [`install_sqlite_rule`] when it's done.
pub(crate) fn sqlite_rule(action: &AuthAction<'_>) -> Authorization {
    match action {
        AuthAction::Attach { filename: "" } => Authorization::Allow,
        AuthAction::Attach { .. } => Authorization::Deny,
        AuthAction::Unknown {
            code: ffi::SQLITE_ATTACH,
            ..
        } => Authorization::Deny,
        AuthAction::Pragma {
            pragma_name,
            pragma_value: Some(_),
        } if REDIRECTING_PRAGMAS
            .iter()
            .any(|p| p.eq_ignore_ascii_case(pragma_name)) =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
}

/// Settings that move SQLite's temp or data files somewhere else.
const REDIRECTING_PRAGMAS: &[&str] =
    &["temp_store", "temp_store_directory", "data_store_directory"];

/// Makes [`sqlite_rule`] the connection's authorizer.
pub(crate) fn install_sqlite_rule(conn: &Connection) -> rusqlite::Result<()> {
    conn.authorizer(Some(|ctx: AuthContext<'_>| sqlite_rule(&ctx.action)))
}

/// How many names (hard links) the open file has.
fn link_count(file: &File) -> io::Result<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        // SAFETY: the handle is valid while `file` lives, and `info` is a
        // plain struct Windows fills in.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(u64::from(info.nNumberOfLinks))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(file.metadata()?.nlink())
    }
}

/// A guarded path for a test database at `rel` under `root`, which becomes
/// the guard's app data root (it's created if missing).
#[cfg(test)]
pub(crate) fn test_path(root: &Path, rel: &str) -> GuardedPath {
    WriteGuard::app_data(root)
        .unwrap()
        .check(&root.join(rel))
        .unwrap()
}

/// `path` made absolute-looking and tidy without touching the disk: `.` and
/// `..` resolved, and on Windows the `\\?\` form with every name literal.
fn lexical(path: &Path) -> Result<PathBuf, GuardError> {
    #[cfg(windows)]
    {
        crate::paths::verbatim_absolute(path).map_err(|_| GuardError::NotAbsolute(path.into()))
    }
    #[cfg(not(windows))]
    {
        use std::path::Component;
        if !path.is_absolute() {
            return Err(GuardError::NotAbsolute(path.into()));
        }
        let mut out = PathBuf::new();
        for part in path.components() {
            match part {
                Component::Prefix(_) | Component::RootDir => out.push(part.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !out.pop() {
                        return Err(GuardError::NotAbsolute(path.into()));
                    }
                }
                Component::Normal(name) => out.push(name),
            }
        }
        Ok(out)
    }
}

/// Where a tidy absolute path really lands: its deepest existing part is
/// resolved on disk (links and junctions followed) and the missing rest is
/// added back. An existing part that can't be resolved, such as a link to
/// nowhere, is an error: writing through it could land anywhere.
fn real_path(path: &Path) -> Result<PathBuf, GuardError> {
    let unresolvable = |source| GuardError::Unresolvable {
        path: path.into(),
        source,
    };
    let mut missing = Vec::new();
    let mut existing = path;
    // `symlink_metadata` doesn't follow a link, so a dangling link counts
    // as existing, and then fails to resolve below.
    while let Err(e) = fs::symlink_metadata(existing) {
        if e.kind() != io::ErrorKind::NotFound {
            return Err(unresolvable(e));
        }
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return Err(unresolvable(e));
        };
        missing.push(name.to_owned());
        existing = parent;
    }
    let mut real = fs::canonicalize(existing).map_err(unresolvable)?;
    real.extend(missing.iter().rev());
    lexical(&real)
}

/// Whether `path` is `root` or inside it. Both are already resolved.
fn within(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        crate::paths::is_within(path, root)
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

/// False for a drive or filesystem root (`C:\`, `/`), which is never an
/// acceptable place to write freely.
fn has_a_folder_name(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::Normal(_)))
}
