//! A simple fallback for macOS and Linux, which are built in CI but not
//! supported (ROADMAP §1.1). The identity is the device number, which is
//! not stable across reboots; good enough to compile and test against.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::{Volume, VolumeId, VolumeKind};

/// Finds the volume `path` lives on. The path must exist.
pub fn volume_for(path: &Path) -> io::Result<Volume> {
    let path = std::fs::canonicalize(path)?;
    let dev = std::fs::metadata(&path)?.dev();
    // The mount point is the highest ancestor on the same device.
    let mut mount_path = path.clone();
    while let Some(parent) = mount_path.parent() {
        if std::fs::metadata(parent)?.dev() != dev {
            break;
        }
        mount_path = parent.to_path_buf();
    }
    let label = mount_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Volume {
        id: VolumeId(format!("dev={dev:x}")),
        label,
        mount_path,
        kind: VolumeKind::Internal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_dir_resolves_to_a_volume_that_contains_it() {
        let dir = tempfile::tempdir().unwrap();
        let volume = volume_for(dir.path()).unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        assert!(canonical.starts_with(&volume.mount_path), "{volume:?}");
        assert!(!volume.id.as_str().is_empty());
    }

    #[test]
    fn every_path_on_one_volume_gives_the_same_volume() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            volume_for(&nested).unwrap(),
            volume_for(dir.path()).unwrap()
        );
    }
}
