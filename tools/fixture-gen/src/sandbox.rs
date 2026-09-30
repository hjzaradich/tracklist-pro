//! The only way the generator touches the disk.
//!
//! A [`Target`] is a folder given on the command line. It must be new or
//! empty, and every write goes through a [`RelPath`]: a path relative to the
//! target whose parts can't climb out of it (`..`), jump elsewhere (a drive
//! or root), or smuggle a separator. Files are created, never overwritten.
//!
//! On Windows every path is a `\\?\` path, so names Windows would otherwise
//! alter (a trailing dot or space, `CON`) are written exactly as given, and
//! can't land on a sibling or a device.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum SandboxError {
    /// The target exists and has something in it.
    NotEmpty(PathBuf),
    /// The target exists but isn't a folder.
    NotAFolder(PathBuf),
    /// The target's parent folder doesn't exist. We create the target
    /// itself, but nothing above it.
    NoParent(PathBuf),
    /// A relative path that would leave the target, or isn't a valid name.
    BadPath(String),
    Io(io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::NotEmpty(p) => write!(
                f,
                "{} is not empty; give an empty or new folder",
                p.display()
            ),
            SandboxError::NotAFolder(p) => write!(f, "{} is not a folder", p.display()),
            SandboxError::NoParent(p) => write!(f, "the parent of {} doesn't exist", p.display()),
            SandboxError::BadPath(why) => write!(f, "refused path: {why}"),
            SandboxError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SandboxError {}

impl From<io::Error> for SandboxError {
    fn from(e: io::Error) -> Self {
        SandboxError::Io(e)
    }
}

/// A path relative to the target, one validated name per part.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(Vec<String>);

impl RelPath {
    /// Refuses anything that could resolve outside the target: `..`, `.`,
    /// empty parts, separators, drive letters (`C:`), and characters no
    /// Windows file name may hold.
    pub fn new<S: AsRef<str>>(parts: &[S]) -> Result<RelPath, SandboxError> {
        if parts.is_empty() {
            return Err(SandboxError::BadPath("empty path".into()));
        }
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            let part = part.as_ref();
            let bad = |why: &str| Err(SandboxError::BadPath(format!("{part:?}: {why}")));
            if part.is_empty() {
                return bad("empty name");
            }
            if part == "." || part == ".." {
                return bad("relative step");
            }
            if let Some(c) = part.chars().find(|&c| {
                matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
                    || (c as u32) < 0x20
            }) {
                return bad(&format!("contains {c:?}"));
            }
            out.push(part.to_owned());
        }
        Ok(RelPath(out))
    }

    /// Splits a `/`-separated path. Each part is checked as in [`RelPath::new`].
    pub fn parse(path: &str) -> Result<RelPath, SandboxError> {
        RelPath::new(&path.split('/').collect::<Vec<_>>())
    }

    pub fn parts(&self) -> &[String] {
        &self.0
    }

    pub fn parent(&self) -> Option<RelPath> {
        (self.0.len() > 1).then(|| RelPath(self.0[..self.0.len() - 1].to_vec()))
    }

    pub fn file_name(&self) -> &str {
        self.0.last().expect("never empty")
    }

    pub fn join(&self, name: &str) -> Result<RelPath, SandboxError> {
        let mut parts = self.0.clone();
        parts.push(name.to_owned());
        RelPath::new(&parts)
    }

    /// `/`-separated, as the manifest records it.
    pub fn to_manifest(&self) -> String {
        self.0.join("/")
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_manifest())
    }
}

/// The folder the generator writes into, and nowhere else.
#[derive(Debug)]
pub struct Target {
    root: PathBuf,
}

impl Target {
    /// Opens `path` for writing: creates it if it's missing (its parent must
    /// exist), accepts it if it's an empty folder, refuses it otherwise.
    pub fn create(path: &Path) -> Result<Target, SandboxError> {
        match fs::metadata(path) {
            Ok(m) if !m.is_dir() => return Err(SandboxError::NotAFolder(path.into())),
            Ok(_) => {
                if fs::read_dir(path)?.next().is_some() {
                    return Err(SandboxError::NotEmpty(path.into()));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let parent = std::path::absolute(path)?
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| SandboxError::NoParent(path.into()))?;
                if !parent.is_dir() {
                    return Err(SandboxError::NoParent(path.into()));
                }
                fs::create_dir(path)?;
            }
            Err(e) => return Err(e.into()),
        }
        // canonicalize gives a `\\?\` path on Windows.
        let root = fs::canonicalize(path)?;
        Ok(Target { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The on-disk path for `rel`. Only [`RelPath`] can be joined, so the
    /// result is always inside the root.
    pub fn resolve(&self, rel: &RelPath) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.parts() {
            p.push(part);
        }
        p
    }

    /// Creates a folder (and any missing folders above it, inside the target).
    pub fn create_dir(&self, rel: &RelPath) -> Result<(), SandboxError> {
        let mut p = self.root.clone();
        for part in rel.parts() {
            p.push(part);
            match fs::create_dir(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && p.is_dir() => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Writes a new file. Fails if anything already exists at `rel`.
    pub fn write_new(&self, rel: &RelPath, bytes: &[u8]) -> Result<(), SandboxError> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.resolve(rel))?;
        f.write_all(bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_refuse_every_way_out_of_the_target() {
        for bad in [
            "..",
            "a/../..",
            ".",
            "",
            "a//b",
            "C:",
            "C:evil.mp3",
            "a\\..\\..\\b",
            "\\\\server\\share",
            "a\u{0}b",
            "what?.mp3",
        ] {
            assert!(RelPath::parse(bad).is_err(), "{bad:?} was accepted");
        }
        assert!(RelPath::new(&["/etc"]).is_err());
    }

    #[test]
    fn awkward_but_legal_names_are_accepted() {
        for ok in [
            "Q.X.Z.",
            "Artist ",
            "CON.mp3",
            "a # % + & ' b",
            "東京",
            "🔥",
        ] {
            assert!(RelPath::new(&[ok]).is_ok(), "{ok:?} was refused");
        }
    }
}
