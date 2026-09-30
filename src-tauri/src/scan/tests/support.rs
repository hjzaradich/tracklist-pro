#![cfg(test)]
//! Test helpers: a migrated database in a temp dir, and a made-up volume
//! mounted at a temp dir, so tests never need a real drive.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::{ReadPool, Writer};
use crate::paths::Volumes;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

/// A migrated database in a temp dir, with its writer and read pool.
pub(super) fn db() -> (tempfile::TempDir, Writer, ReadPool) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    (dir, writer, reads)
}

/// A made-up volume identity, built the way real ones are.
pub(super) fn serial(n: u32) -> VolumeId {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(n),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
}

/// One volume "mounted" at a real temp folder. Paths are canonicalized on
/// disk like the real lookup does, so junctions resolve. Clones share one
/// plugged-in switch.
#[derive(Clone)]
pub(super) struct TempVolume {
    pub id: VolumeId,
    pub label: String,
    /// The mount point as the OS spells it (`\\?\C:\…` on Windows).
    pub mount: PathBuf,
    online: Arc<AtomicBool>,
}

impl TempVolume {
    pub(super) fn new(mount: &Path, n: u32) -> TempVolume {
        TempVolume {
            id: serial(n),
            label: format!("TEST{n}"),
            mount: fs::canonicalize(mount).unwrap(),
            online: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Unplugs (or plugs back) the volume.
    pub(super) fn set_online(&self, online: bool) {
        self.online.store(online, Ordering::SeqCst);
    }
}

impl Volumes for TempVolume {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        if !path.starts_with(&self.mount) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} isn't on the test volume", path.display()),
            ));
        }
        Ok(Volume {
            id: self.id.clone(),
            label: self.label.clone(),
            mount_path: self.mount.clone(),
            kind: VolumeKind::External,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (self.online.load(Ordering::SeqCst) && *id == self.id).then(|| self.mount.clone())
    }
}

/// A junction (Windows) or symlink (elsewhere) at `link` pointing at the
/// folder `target`. Junctions need no special privilege.
pub(super) fn link_dir(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink: {made:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}
