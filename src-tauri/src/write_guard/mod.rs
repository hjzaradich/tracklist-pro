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
//! connection, reader or writer, is opened from a [`GuardedPath`]. SQLite is
//! given the checked path in its plain drive-letter spelling where Win32
//! reads that as the same file, because it takes a `\\?\` name for a network
//! share and locks that kind of database in a way that can go wrong
//! (`GuardedPath::sqlite_name`).

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{ffi, Connection, OpenFlags};

#[cfg(test)]
pub(crate) mod no_writes_elsewhere;
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
    /// [`WriteGuard::save_new_bundle_outside`] refused, for this reason.
    SaveRefused { why: SaveRefusal, path: PathBuf },
}

/// Why [`WriteGuard::save_new_bundle_outside`] refused. One kind per rule,
/// so a refusal says exactly which condition failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveRefusal {
    /// The source isn't a bundle file the app made in `bundles/`.
    SourceNotABundle,
    /// The destination's name doesn't end in `.zip`.
    NotZip,
    /// A name Windows reads as something else: a `:` (an alternate data
    /// stream), a device name (`CON`, `NUL`, `COM1`...), or a name ending
    /// in a dot or a space.
    BadName,
    /// Something is already at the destination (file, folder or link).
    Exists,
    /// The destination's folder doesn't exist.
    FolderMissing,
    /// The destination's folder, or a folder above it, is a link or
    /// junction, or isn't a folder.
    FolderNotPlain,
    /// Inside (or equal to) a scan or Inbox folder.
    InMusicFolder,
    /// Inside (or equal to) the app data folder.
    InAppData,
    /// On a drive that looks like a rekordbox gig stick.
    GigStick,
    /// Inside rekordbox's own data folder.
    InRekordboxFolder,
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
            GuardError::SaveRefused { why, path } => {
                write!(f, "won't save to {}: {why:?}", path.display())
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

    /// [`Self::check`], refusing a root itself: for writes that make or
    /// delete files beside `path`, in its own folder. A root's own folder
    /// is outside it.
    fn check_below_a_root(&self, path: &Path) -> Result<GuardedPath, GuardError> {
        let checked = self.check(path)?;
        if self
            .roots
            .iter()
            .any(|(_, root)| within(root, &checked.path))
        {
            return Err(GuardError::IsRoot(checked.path));
        }
        Ok(checked)
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

    /// Adds `bytes` to the end of the file at `path`, creating it if needed.
    /// The same checks as [`Self::write`]; nothing already in the file is
    /// changed. A write cut short leaves a partial tail, which is the
    /// caller's to tolerate.
    pub fn append(&self, path: &Path, bytes: &[u8]) -> Result<(), GuardError> {
        use std::io::{Seek, SeekFrom, Write};
        let checked = self.check(path)?;
        let mut file = checked.open_one_name(true)?;
        file.seek(SeekFrom::End(0))
            .and_then(|_| file.write_all(bytes))
            .map_err(|e| checked.io(e))
    }

    /// Writes `bytes` as the whole file at `path` without ever leaving a
    /// half-written file there: the bytes go to a new file beside it, are
    /// flushed to disk, and that file is then renamed over `path`. On any
    /// failure the new file is deleted, and `path` is as it was (missing,
    /// or still holding its old content).
    ///
    /// `path` passes the same checks as [`Self::write`], and can't be a
    /// root itself (the file beside it would be outside). The file beside
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
        // The new file is made beside `path`, so `path` can't be a root.
        let checked = self.check_below_a_root(path)?;
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

    /// Deletes the files an earlier [`Self::write_then_rename`] to `path`
    /// could have left when it was cut short (a crash, a power cut):
    /// exactly the names it can give its new file ([`beside_names`]), in
    /// `path`'s own folder. Only a plain file is deleted; a folder or a
    /// link with such a name is left alone, and so is `path` itself.
    /// Returns how many were deleted.
    ///
    /// `path` passes the same check as a write and must be below a root,
    /// never a root itself (whose own folder is outside it), and every
    /// name is checked to be inside the root before it's looked at. So
    /// nothing outside a root is looked at or deleted.
    pub fn remove_leftover_parts(&self, path: &Path) -> Result<usize, GuardError> {
        let checked = self.check_below_a_root(path)?;
        let mut removed = 0;
        for name in beside_names(&checked.path) {
            let leftover = checked.path.with_file_name(name);
            if !self.roots.iter().any(|(_, root)| within(&leftover, root)) {
                return Err(GuardError::Outside(leftover));
            }
            // Not following links: only a plain file is one of ours.
            match fs::symlink_metadata(&leftover) {
                Ok(meta) if meta.file_type().is_file() => {}
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(GuardError::Io {
                        path: leftover,
                        source,
                    })
                }
            }
            match fs::remove_file(&leftover) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(GuardError::Io {
                        path: leftover,
                        source,
                    })
                }
            }
        }
        Ok(removed)
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

    /// Saves `bytes` as the file `name` in `folder` (made if missing), whole
    /// or not at all ([`Self::write_then_rename`]), then deletes the oldest
    /// plain files in `folder` that `is_ours` accepts until `keep` are left.
    /// Names are compared as text, so they must sort oldest first (a
    /// timestamp). A file `is_ours` doesn't accept, a folder and a link are
    /// never deleted. Returns the saved file's path.
    ///
    /// `name` is one plain name, never a path, and `folder` must be inside a
    /// root. The file just saved is never deleted, whatever the order of the
    /// names. A file that can't be deleted is left; the next save tries again.
    pub fn save_keeping_newest(
        &self,
        folder: &Path,
        name: &str,
        bytes: &[u8],
        keep: usize,
        is_ours: impl Fn(&str) -> bool,
    ) -> Result<PathBuf, GuardError> {
        let folder = self.create_dir_all(folder)?;
        let path = folder.as_path().join(name);
        // Exactly one plain name: not ".", "..", a drive, a root or a path.
        if !matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) {
            return Err(GuardError::Outside(path));
        }
        self.write_then_rename(&path, bytes)?;
        let saved = name;
        let mut names: Vec<String> = fs::read_dir(folder.as_path())
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        let entry = entry.ok()?;
                        let name = entry.file_name().into_string().ok()?;
                        // The file just saved is never a candidate, whatever the clock says.
                        (entry.file_type().ok()?.is_file() && is_ours(&name) && name != saved)
                            .then_some(name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        // The saved file counts as one of the `keep`.
        let extra = names.len().saturating_sub(keep.saturating_sub(1));
        for old in names.iter().take(extra) {
            let _ = self.remove_file(&folder.as_path().join(old));
        }
        Ok(path)
    }

    /// Deletes a folder and everything in it. Links (the folder itself, or
    /// any inside it) are removed, never followed, so nothing they point to
    /// is touched. A root can't be deleted.
    pub fn remove_dir_all(&self, path: &Path) -> Result<(), GuardError> {
        let checked = self.check_entry(path)?;
        fs::remove_dir_all(&checked.path).map_err(|e| checked.io(e))
    }
}

/// Folders a bundle must never be saved in or on, besides app data.
#[derive(Debug, Default, Clone)]
pub struct KeepOut {
    /// Every scan and Inbox folder, where they are (or were last seen).
    pub music_folders: Vec<PathBuf>,
    /// rekordbox's own data folders (`Pioneer` in the roaming and in the
    /// local config folders), where known.
    pub rekordbox_data: Vec<PathBuf>,
}

impl WriteGuard {
    /// THE ONE EXCEPTION to "the app writes only inside its roots"
    /// (ROADMAP 4.1, owner decision 2026-11-07): copies one bundle the app
    /// made (`source`, a file in app data's `bundles/`) to a NEW file the
    /// user named in a Save dialog. It is the only way the app writes
    /// outside app data, and it writes exactly one file, only when ALL hold:
    ///
    /// 1. `source` is a plain file named `tracklist-pro-*.zip` directly in
    ///    app data's `bundles/` (else `SourceNotABundle`);
    /// 2. `dest` ends in `.zip` (`NotZip`), and no part of it after the drive
    ///    holds a `:`, is a device name or ends in a dot or space (`BadName`);
    /// 3. nothing at all is at `dest`, not even a link (`Exists`); the file
    ///    is made with create-new, so it is never replaced or followed;
    /// 4. `dest`'s folder exists (`FolderMissing`) and neither it nor any
    ///    folder above it is a symlink or junction, and it is a folder
    ///    (`FolderNotPlain`);
    /// 5. `dest` is not inside, or equal to, a music folder
    ///    (`InMusicFolder`) or app data (`InAppData`);
    /// 6. the drive root holds no `PIONEER` or `Contents` entry, and no
    ///    folder on `dest`'s path holds both (a stick mounted in a folder):
    ///    the marks of a gig stick (`GigStick`). Mount points are links, so refused by rule 4;
    /// 7. `dest` is not inside rekordbox's data folder (`InRekordboxFolder`).
    ///
    /// The bytes are streamed into the created file. If that fails, only
    /// the file this call created is removed.
    pub fn save_new_bundle_outside(
        &self,
        source: &Path,
        dest: &Path,
        keep_out: &KeepOut,
    ) -> Result<(), GuardError> {
        let refuse = |why| GuardError::SaveRefused {
            why,
            path: source.into(),
        };
        let bundles = self.check(&self.app_data_dir().join("bundles"))?;
        let source = self
            .check(source)
            .map_err(|_| refuse(SaveRefusal::SourceNotABundle))?;
        let name = source.path.file_name().and_then(|n| n.to_str());
        let is_bundle_name =
            name.is_some_and(|n| n.starts_with("tracklist-pro-") && n.ends_with(".zip"));
        if source.path.parent() != Some(bundles.path.as_path()) || !is_bundle_name {
            return Err(refuse(SaveRefusal::SourceNotABundle));
        }
        let mut from = File::open(&source.path).map_err(|e| source.io(e))?;
        if !from.metadata().is_ok_and(|m| m.is_file()) {
            return Err(refuse(SaveRefusal::SourceNotABundle));
        }
        self.save_stream(&mut from, dest, keep_out, drive_root, &|_| {})
    }

    /// Rules 2 to 7 of [`Self::save_new_bundle_outside`], then the copy.
    /// `root_of` finds the drive root (a test names a temp folder instead).
    fn save_stream(
        &self,
        from: &mut dyn io::Read,
        dest: &Path,
        keep_out: &KeepOut,
        root_of: fn(&Path) -> Option<PathBuf>,
        before_create: &dyn Fn(&Path),
    ) -> Result<(), GuardError> {
        // Before anything resolves the name: a `:` may be refused as a path.
        if has_a_bad_name(dest) {
            return Err(GuardError::SaveRefused {
                why: SaveRefusal::BadName,
                path: dest.into(),
            });
        }
        let tidy = lexical(dest)?;
        let refuse = |why, path: &Path| GuardError::SaveRefused {
            why,
            path: path.into(),
        };
        if has_a_bad_name(&tidy) {
            return Err(refuse(SaveRefusal::BadName, &tidy));
        }
        let is_zip = tidy
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if !is_zip || tidy.file_stem().is_none() {
            return Err(refuse(SaveRefusal::NotZip, &tidy));
        }
        if fs::symlink_metadata(&tidy).is_ok() {
            return Err(refuse(SaveRefusal::Exists, &tidy));
        }
        let Some(folder) = tidy.parent() else {
            return Err(refuse(SaveRefusal::FolderMissing, &tidy));
        };
        match fs::symlink_metadata(folder) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(refuse(SaveRefusal::FolderNotPlain, &tidy)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(refuse(SaveRefusal::FolderMissing, &tidy));
            }
            Err(source) => {
                return Err(GuardError::Unresolvable {
                    path: tidy.clone(),
                    source,
                })
            }
        }
        // `symlink_metadata` doesn't follow; std counts a junction as a link.
        if folder
            .ancestors()
            .any(|a| fs::symlink_metadata(a).is_ok_and(|m| m.file_type().is_symlink()))
        {
            return Err(refuse(SaveRefusal::FolderNotPlain, &tidy));
        }
        let real = real_path(&tidy)?;
        if self.roots.iter().any(|(_, root)| within(&real, root)) {
            return Err(refuse(SaveRefusal::InAppData, &real));
        }
        if inside_any(&real, keep_out.music_folders.iter().map(PathBuf::as_path)) {
            return Err(refuse(SaveRefusal::InMusicFolder, &real));
        }
        if real.ancestors().skip(1).any(looks_like_a_stick)
            || root_of(&real)
                .is_some_and(|root| holds_entry(&root, "Contents") || holds_entry(&root, "PIONEER"))
        {
            return Err(refuse(SaveRefusal::GigStick, &real));
        }
        if inside_any(&real, keep_out.rekordbox_data.iter().map(PathBuf::as_path)) {
            return Err(refuse(SaveRefusal::InRekordboxFolder, &real));
        }
        // Create-new: fails if anything appeared meanwhile, never replaces.
        before_create(&real);
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&real)
            .map_err(|e| match e.kind() {
                io::ErrorKind::AlreadyExists => refuse(SaveRefusal::Exists, &real),
                _ => GuardError::Io {
                    path: real.clone(),
                    source: e,
                },
            })?;
        let copied = io::copy(from, &mut file).and_then(|_| file.sync_all());
        drop(file);
        copied.map_err(|source| {
            // Only the file this call just created.
            let _ = fs::remove_file(&real);
            GuardError::Io {
                path: real.clone(),
                source,
            }
        })
    }
}

/// Whether `path` (resolved) is one of `folders` or inside one, going by
/// each folder as written and as it resolves on disk.
fn inside_any<'a>(path: &Path, mut folders: impl Iterator<Item = &'a Path>) -> bool {
    folders.any(|folder| {
        let written = lexical(folder).ok();
        let resolved = written.as_deref().and_then(|w| real_path(w).ok());
        [written, resolved]
            .iter()
            .flatten()
            .any(|f| within(path, f))
    })
}

/// The drive (or share) root `path` is on: its prefix and root directory.
fn drive_root(path: &Path) -> Option<PathBuf> {
    let mut root = PathBuf::new();
    for part in path.components().take(2) {
        match part {
            Component::Prefix(_) | Component::RootDir => root.push(part.as_os_str()),
            _ => break,
        }
    }
    root.has_root().then_some(root)
}

/// Whether `folder` holds an entry called `name`, in any case.
fn holds_entry(folder: &Path, name: &str) -> bool {
    fs::read_dir(folder).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
    })
}

/// Whether `folder` holds both of rekordbox's marks on a stick, `PIONEER`
/// and `Contents`: a stick mounted inside a folder. (`PIONEER` alone is
/// not enough: `Documents\Pioneer` is an ordinary folder.)
fn looks_like_a_stick(folder: &Path) -> bool {
    holds_entry(folder, "PIONEER") && holds_entry(folder, "Contents")
}

/// A name Windows would read as something other than a plain file or
/// folder, in any part of `path` after its drive or share: a `:` (stream
/// syntax such as `x.zip:s`), a device name with or without an extension,
/// or a name ending in a dot or a space.
fn has_a_bad_name(path: &Path) -> bool {
    path.components().any(|part| {
        let Component::Normal(name) = part else {
            return false;
        };
        let name = name.to_string_lossy();
        let stem = name
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        // `COM0` to `COM9` and the superscript digits Windows also reserves.
        let numbered = |prefix: &str| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                let mut chars = n.chars();
                matches!(
                    (chars.next(), chars.next()),
                    (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
                )
            })
        };
        name.contains(':')
            || name.ends_with('.')
            || name.ends_with(' ')
            || ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
            || numbered("COM")
            || numbered("LPT")
    })
}

/// The folder in app data that holds every [`Staging`] folder.
pub const STAGING_DIR: &str = "staging";

/// A folder of files that live only while one task runs: `staging\<name>`
/// in app data. Made empty (anything a crash left is deleted first), and
/// deleted with everything in it by [`Staging::close`], or when dropped.
pub struct Staging {
    guard: WriteGuard,
    dir: PathBuf,
    closed: bool,
}

impl WriteGuard {
    /// An empty staging folder called `name` (one plain name, no
    /// separators).
    pub fn staging(&self, name: &str) -> Result<Staging, GuardError> {
        let dir = self
            .app_data_dir()
            .join(STAGING_DIR)
            .join(plain_name(name)?);
        let checked = self.check_entry(&dir)?;
        if fs::symlink_metadata(&checked.path).is_ok() {
            fs::remove_dir_all(&checked.path).map_err(|e| checked.io(e))?;
        }
        let dir = self.create_dir_all(&dir)?.path;
        Ok(Staging {
            guard: self.clone(),
            dir,
            closed: false,
        })
    }
}

impl Staging {
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Streams everything `from` reads into a new file `name` in this
    /// folder, atomically: the file appears whole, flushed to disk, or not
    /// at all ([`WriteGuard::write_then_rename`]). Returns its path.
    pub fn copy_in(&self, name: &str, from: &mut dyn io::Read) -> Result<PathBuf, GuardError> {
        let path = self.dir.join(plain_name(name)?);
        self.guard.write_then_rename_with(
            &path,
            |file| {
                let mut out = io::BufWriter::with_capacity(1 << 16, file);
                io::copy(from, &mut out)?;
                io::Write::flush(&mut out)
            },
            || Ok(()),
        )?;
        Ok(path)
    }

    /// Creates the file `name` in this folder and opens it for writing.
    pub fn create(&self, name: &str) -> Result<(PathBuf, File), GuardError> {
        let path = self.dir.join(plain_name(name)?);
        let file = self.guard.create_file(&path)?;
        Ok((path, file))
    }

    /// Deletes the folder and everything in it. Anything still open in it
    /// must be closed first.
    pub fn close(mut self) -> Result<(), GuardError> {
        self.closed = true;
        self.guard.remove_dir_all(&self.dir)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.guard.remove_dir_all(&self.dir);
        }
    }
}

/// `name` if it's one plain file or folder name.
fn plain_name(name: &str) -> Result<&str, GuardError> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) if !name.contains(['/', '\\']) => Ok(name),
        _ => Err(GuardError::NotAbsolute(PathBuf::from(name))),
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

    /// The name SQLite is given for the database file: the path with the
    /// `\\?\` prefix taken off where that changes nothing (see
    /// [`plain_drive_spelling`]).
    ///
    /// SQLite's Windows file layer treats a name that starts with `\\` as a
    /// network share and then takes every shared-memory lock through one
    /// shared handle. With that, two connections taking the same read lock
    /// of the `-shm` file at the same moment (two reads beginning together
    /// while the log is empty) can leave one lock held for good: nothing
    /// SQLite tracks holds it, closing the connections doesn't free it, and
    /// from then on no checkpoint can move a frame into the database. The
    /// log grows with every write for the rest of the session, and the
    /// checkpoint at the app's exit finds it blocked. Every connection must
    /// be opened under the same name, so this is the only place that names
    /// the file to SQLite.
    fn sqlite_name(&self) -> PathBuf {
        plain_drive_spelling(&self.path).unwrap_or_else(|| self.path.clone())
    }

    /// Opens (or creates) the SQLite database at this path, read-write.
    ///
    /// SQLite puts its `-wal`, `-shm` and journal files beside the database,
    /// so they stay inside the root too. Temp tables, indexes and sorts are
    /// kept in memory (`temp_store = MEMORY`) rather than spilled to the
    /// system temp folder, and `ATTACH` / `VACUUM INTO` are refused.
    pub fn open_database(&self) -> rusqlite::Result<Connection> {
        self.database_files_have_one_name()?;
        confine(Connection::open(self.sqlite_name())?)
    }

    /// Opens the SQLite database at this path read-only, confined like
    /// [`Self::open_database`]. It must already exist.
    pub fn open_database_read_only(&self) -> rusqlite::Result<Connection> {
        self.database_files_have_one_name()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        confine(Connection::open_with_flags(self.sqlite_name(), flags)?)
    }

    /// The bundle and Library backup rule (ROADMAP 4.1): fills the new
    /// database at this path with a copy of the whole database `from` is
    /// open on, through SQLite's online backup, and returns the copy open.
    /// The copy is read in one read transaction, so it is one consistent
    /// moment of the Library even while the writer works.
    ///
    /// The one way app code copies a database. The copy is written only
    /// here, at this checked path, on a connection confined like
    /// [`Self::open_database`]. `VACUUM INTO` stays refused: it names its
    /// file in SQL, past every check. Whatever database is at the path is
    /// overwritten, so it must be a file in a [`Staging`] folder
    /// (`staging\<name>\<file>`) and never named like the Library's own
    /// file: the Library is never written over by a copy.
    pub fn copy_database_from(&self, from: &Connection) -> rusqlite::Result<Connection> {
        use rusqlite::backup::{Backup, StepResult};
        let in_staging = self
            .path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name == STAGING_DIR);
        if !in_staging || self.path.file_name() == Some(crate::db::DB_FILE_NAME.as_ref()) {
            return Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::SQLITE_CANTOPEN),
                Some(format!(
                    "{} isn't a staging file, so no copy is written there",
                    self.path.display()
                )),
            ));
        }
        let mut to = self.open_database()?;
        // -1: every page in one step, so in one read transaction of `from`.
        let step = Backup::new(from, &mut to)?.step(-1)?;
        if !matches!(step, StepResult::Done) {
            return Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::SQLITE_BUSY),
                Some(format!("the copy stopped early ({step:?})")),
            ));
        }
        Ok(to)
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
        let mut last = (self.path.clone(), io::ErrorKind::AlreadyExists.into());
        for temp_name in beside_names(&self.path) {
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

/// Every name [`GuardedPath::create_beside`] can give the new file it makes
/// beside `path`, in the order it tries them: `<name>.part`, then
/// `<name>.1.part`, `<name>.2.part`, … The one list both the write and
/// [`WriteGuard::remove_leftover_parts`] go by, so they can't disagree.
fn beside_names(path: &Path) -> impl Iterator<Item = std::ffi::OsString> {
    let name = path.file_name().unwrap_or_default().to_owned();
    (0..BESIDE_ATTEMPTS).map(move |attempt| {
        let mut temp_name = name.clone();
        if attempt > 0 {
            temp_name.push(format!(".{attempt}"));
        }
        temp_name.push(".part");
        temp_name
    })
}

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

/// `\\?\C:\dir\name` as `C:\dir\name`, when Windows reads the two as the
/// same file: a drive path whose names Win32 wouldn't rewrite (no trailing
/// dot or space in any, no device name such as `CON` or `AUX.db`, no
/// character it forbids) and that is short enough for a plain path. `None`
/// for any other path, which is then used as it is. Plain string logic, so
/// the tests run on every platform.
fn plain_drive_spelling(path: &Path) -> Option<PathBuf> {
    /// Characters of the plain path (bytes, so a non-ASCII name counts
    /// more). Windows' limit is 260 including the terminating NUL, and the
    /// longest name SQLite adds is `-journal` (8): 260 - 1 - 8 = 251 would
    /// still fit; 240 keeps some room over that.
    const LONGEST_PLAIN: usize = 240;
    const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

    let rest = path.to_str()?.strip_prefix(r"\\?\")?;
    let b = rest.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return None;
    }
    if rest.len() > LONGEST_PLAIN {
        return None;
    }
    for name in rest[3..].split('\\') {
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = DEVICES.contains(&stem.as_str())
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit());
        let forbidden = name
            .chars()
            .any(|c| c.is_control() || "<>:\"/|?*".contains(c));
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.ends_with('.')
            || name.ends_with(' ')
            || device
            || forbidden
        {
            return None;
        }
    }
    Some(PathBuf::from(rest))
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
