//! The real [`Volumes`]: asks Windows which volumes are mounted and where.
//!
//! Only reads. Nothing here opens a file; the volume queries go through
//! [`crate::volume`].
//!
//! A drive letter says nothing about which drive is behind it, so the
//! mounted volumes are looked at again whenever drives may have come or
//! gone: the device watch (1aB-9) calls [`devices_changed`], and every
//! [`SystemVolumes`] looks again before its next answer. A drive swapped
//! for another on the same letter then never answers for the old one.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_MORE_DATA, INVALID_HANDLE_VALUE, MAX_PATH,
};
use windows_sys::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumePathNamesForVolumeNameW,
};

use super::{parse_absolute, Volumes};
use crate::volume::{self, CloneClash, Remembered, Sighting, Volume, VolumeId};

/// Goes up each time drives may have come or gone.
static DEVICES: AtomicU64 = AtomicU64::new(0);

/// Says drives may have come or gone (a device arrived or was removed):
/// every [`SystemVolumes`] looks at the mounted volumes again before it
/// next answers.
pub fn devices_changed() {
    DEVICES.fetch_add(1, Ordering::SeqCst);
}

/// Lists what's mounted now: each mount point's volume and GUID.
type Sightings = Box<dyn Fn() -> Vec<Sighting> + Send + Sync>;

/// The mounted volumes, as of [`SystemVolumes::scan`] or the last device
/// change since. Resolving many paths reuses one look.
pub struct SystemVolumes {
    mounted: RwLock<Mounted>,
    /// What the library remembers about each volume, for the clone rule.
    remembered: Vec<Remembered>,
    sightings: Sightings,
    /// [`DEVICES`] in the app; tests move their own.
    devices: &'static AtomicU64,
    /// Whether each network share answered, asked once per look: an
    /// unreachable host takes seconds to time out, and a library on a NAS
    /// that's switched off shouldn't pay that for every track.
    shares: Mutex<HashMap<VolumeId, bool>>,
    share_online: fn(&Path) -> bool,
}

impl std::fmt::Debug for SystemVolumes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemVolumes")
            .field("mounted", &self.mounted_with_guids())
            .finish_non_exhaustive()
    }
}

/// One look at the mounted volumes, after the clone rule.
#[derive(Debug, Default)]
struct Mounted {
    /// The device counter when the look started.
    at: u64,
    /// Each mount point's volume, and the volume's GUID.
    volumes: Vec<(Volume, Option<String>)>,
    clashes: Vec<CloneClash>,
}

impl Mounted {
    fn look(at: u64, sightings: &Sightings, remembered: &[Remembered]) -> Mounted {
        let sightings = sightings();
        let guids: HashMap<PathBuf, Option<String>> = sightings
            .iter()
            .map(|s| (s.volume.mount_path.clone(), s.guid.clone()))
            .collect();
        let scan = volume::tell_clones_apart(sightings, remembered);
        let volumes = scan
            .volumes
            .into_iter()
            .map(|v| {
                let guid = guids.get(&v.mount_path).cloned().flatten();
                (v, guid)
            })
            .collect();
        Mounted {
            at,
            volumes,
            clashes: scan.clashes,
        }
    }

    /// Where the volume `id` is mounted, as the clone rule named it. A clone
    /// the library remembers keeps its `serial=…+guid={…}` identity even
    /// when it's plugged in alone (rule 5 of [`volume::tell_clones_apart`]).
    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        self.volumes
            .iter()
            .find(|(v, _)| &v.id == id)
            .map(|(v, _)| v.mount_path.clone())
    }
}

impl SystemVolumes {
    /// Lists every mounted volume, under each drive letter and mount folder
    /// it has. Volumes that can't be read (a card reader with no card) are
    /// left out, so they resolve as offline. Clones plugged in together
    /// can't be told apart without what the library remembers
    /// ([`SystemVolumes::scan_remembering`]), so none keeps the identity
    /// they share.
    pub fn scan() -> SystemVolumes {
        SystemVolumes::scan_remembering(Vec::new())
    }

    /// [`SystemVolumes::scan`], telling clones apart with what the library
    /// remembers about each volume (the `volume` table's GUID and label).
    pub fn scan_remembering(remembered: Vec<Remembered>) -> SystemVolumes {
        SystemVolumes::from_parts(Box::new(sightings), &DEVICES, remembered, |root| {
            root.is_dir()
        })
    }

    fn from_parts(
        sightings: Sightings,
        devices: &'static AtomicU64,
        remembered: Vec<Remembered>,
        share_online: fn(&Path) -> bool,
    ) -> SystemVolumes {
        let at = devices.load(Ordering::SeqCst);
        SystemVolumes {
            mounted: RwLock::new(Mounted::look(at, &sightings, &remembered)),
            remembered,
            sightings,
            devices,
            shares: Mutex::default(),
            share_online,
        }
    }

    /// Answers from the mounted volumes, looking again first if drives
    /// came or went since the last look.
    fn with_mounted<T>(&self, answer: impl FnOnce(&Mounted) -> T) -> T {
        let now = self.devices.load(Ordering::SeqCst);
        let read = || self.mounted.read().unwrap_or_else(PoisonError::into_inner);
        if read().at != now {
            // The counter was read before looking, so a change during the
            // look makes the next answer look again.
            let fresh = Mounted::look(now, &self.sightings, &self.remembered);
            *self.mounted.write().unwrap_or_else(PoisonError::into_inner) = fresh;
            // A share that didn't answer may be back, and one that did may
            // be gone.
            self.shares
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
        }
        answer(&read())
    }

    /// The mounted volumes, after the clone rule.
    pub fn mounted(&self) -> Vec<Volume> {
        self.with_mounted(|m| m.volumes.iter().map(|(v, _)| v.clone()).collect())
    }

    /// The mounted volumes with their GUIDs, after the clone rule.
    pub fn mounted_with_guids(&self) -> Vec<(Volume, Option<String>)> {
        self.with_mounted(|m| m.volumes.clone())
    }

    /// Volumes plugged in together that share a serial (clones).
    pub fn clashes(&self) -> Vec<CloneClash> {
        self.with_mounted(|m| m.clashes.clone())
    }
}

impl Volumes for SystemVolumes {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    /// The volume holding `path`, named the way the clone rule names it
    /// ([`named`]).
    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        self.sighting_for(path).map(|s| s.volume)
    }

    /// Asks the volume holding `path` who it is right now (serial and
    /// GUID, not a remembered answer), then names it as the clone rule
    /// does ([`named`]).
    fn sighting_for(&self, path: &Path) -> io::Result<Sighting> {
        let found = volume::sighting_for(path)?;
        let guid = found.guid.as_deref();
        let volume =
            self.with_mounted(|m| named(found.volume.clone(), guid, &m.volumes, &m.clashes))?;
        Ok(Sighting {
            volume,
            guid: found.guid,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        // A share is mounted where its identity says, if it answers.
        if let Some(share) = id.as_str().strip_prefix("unc=") {
            let root = PathBuf::from(format!("{share}\\"));
            // Brings the answers about shares up to date with the devices.
            self.with_mounted(|_| ());
            let shares = || self.shares.lock().unwrap_or_else(PoisonError::into_inner);
            let known = shares().get(id).copied();
            // The probe runs without the lock, so one slow share doesn't
            // hold up the others. Two threads may both probe a new share;
            // the first answer is kept.
            let online = known.unwrap_or_else(|| {
                let online = (self.share_online)(&root);
                *shares().entry(id.clone()).or_insert(online)
            });
            return online.then_some(root);
        }
        self.with_mounted(|m| m.mount_path(id))
    }

    fn guid(&self, id: &VolumeId) -> Option<String> {
        self.with_mounted(|m| {
            m.volumes
                .iter()
                .find(|(v, _)| &v.id == id)
                .and_then(|(_, g)| g.clone())
        })
    }
}

/// The volume `found` at a mount point, reporting `guid`, as the clone
/// rule named it in the last look (`mounted`, `clashes`): a clone plugged
/// in with its twin, or one the library remembers, has an identity of its
/// own. Only an entry at the same mount point, with the same serial and
/// the same GUID, names it. A clone that can't be told from its twin can't
/// hold a stored path, so it's refused. A volume the last look didn't see
/// (it arrived since) keeps the identity it reports.
fn named(
    found: Volume,
    guid: Option<&str>,
    mounted: &[(Volume, Option<String>)],
    clashes: &[CloneClash],
) -> io::Result<Volume> {
    if clashes
        .iter()
        .any(|c| c.left_out.contains(&found.mount_path))
    {
        return Err(io::Error::other(format!(
            "{} can't be told apart from its clone",
            found.mount_path.display()
        )));
    }
    let same = |(v, g): &&(Volume, Option<String>)| {
        v.mount_path == found.mount_path
            && g.as_deref() == guid
            && (v.id == found.id || v.id.clone_parts().is_some_and(|(s, _)| s == found.id))
    };
    Ok(mounted
        .iter()
        .find(same)
        .map_or(found.clone(), |(v, _)| v.clone()))
}

/// Every mount point of every local volume, with its volume and GUID.
fn sightings() -> Vec<Sighting> {
    mount_points()
        .iter()
        .filter_map(|mount| volume::sighting_for(&verbatim(mount)?).ok())
        .collect()
}

/// `path` in the `\\?\` form, so a mount folder whose name ends in a dot
/// or space keeps it when it's handed to Win32.
fn verbatim(path: &Path) -> Option<PathBuf> {
    Some(PathBuf::from(
        parse_absolute(path.to_str()?).ok()?.verbatim(),
    ))
}

/// Every drive letter and mount folder of every local volume.
fn mount_points() -> Vec<PathBuf> {
    let mut name = [0u16; MAX_PATH as usize + 1];
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        out.extend(path_names(&name));
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    unsafe { FindVolumeClose(find) };
    out
}

/// The mount points of one volume (`\\?\Volume{…}\`), e.g. `E:\` and
/// `C:\mnt\usb\`. Empty for a volume with none.
fn path_names(volume_name: &[u16]) -> Vec<PathBuf> {
    read_multi_sz(|buf, needed| {
        let ptr = if buf.is_empty() {
            ptr::null_mut()
        } else {
            buf.as_mut_ptr()
        };
        let ok = unsafe {
            GetVolumePathNamesForVolumeNameW(volume_name.as_ptr(), ptr, buf.len() as u32, needed)
        };
        if ok != 0 {
            Ok(())
        } else {
            Err(unsafe { GetLastError() })
        }
    })
}

/// Calls a Win32 function that fills a list of NUL-terminated strings,
/// growing the buffer while it answers `ERROR_MORE_DATA`. The list can grow
/// between the size query and the real call (a drive mounted meanwhile), so
/// one retry isn't always enough. Gives up (empty) on any other error.
fn read_multi_sz(mut call: impl FnMut(&mut [u16], &mut u32) -> Result<(), u32>) -> Vec<PathBuf> {
    let mut buf: Vec<u16> = Vec::new();
    for _ in 0..8 {
        let mut needed = 0u32;
        match call(&mut buf, &mut needed) {
            Ok(()) => {
                return buf
                    .split(|&c| c == 0)
                    .take_while(|s| !s.is_empty())
                    .map(|s| PathBuf::from(OsString::from_wide(s)))
                    .collect()
            }
            Err(ERROR_MORE_DATA) => {
                buf = vec![0; (needed as usize).max(buf.len() * 2).max(1)];
            }
            Err(_) => return Vec::new(),
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{PathError, RelPath, StoredPath};
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    impl SystemVolumes {
        /// These volumes mounted, and drives never come or go.
        fn with(mounted: Vec<Volume>, share_online: fn(&Path) -> bool) -> SystemVolumes {
            static STILL: AtomicU64 = AtomicU64::new(0);
            let sightings: Vec<Sighting> = mounted
                .into_iter()
                .map(|volume| Sighting { volume, guid: None })
                .collect();
            SystemVolumes::from_parts(
                Box::new(move || sightings.clone()),
                &STILL,
                Vec::new(),
                share_online,
            )
        }
    }

    /// Made-up drives the test plugs in and out, and its own device
    /// counter standing in for the device watch.
    struct Desk {
        plugged: Arc<Mutex<Vec<Sighting>>>,
        devices: &'static AtomicU64,
        looks: Arc<AtomicUsize>,
    }

    impl Desk {
        fn new(plugged: Vec<Sighting>) -> Desk {
            Desk {
                plugged: Arc::new(Mutex::new(plugged)),
                devices: Box::leak(Box::new(AtomicU64::new(0))),
                looks: Arc::default(),
            }
        }

        fn volumes(&self, remembered: Vec<Remembered>) -> SystemVolumes {
            let (plugged, looks) = (self.plugged.clone(), self.looks.clone());
            SystemVolumes::from_parts(
                Box::new(move || {
                    looks.fetch_add(1, Ordering::SeqCst);
                    plugged.lock().unwrap().clone()
                }),
                self.devices,
                remembered,
                |_| true,
            )
        }

        /// Swaps what's plugged in, without telling anyone.
        fn plug(&self, now: Vec<Sighting>) {
            *self.plugged.lock().unwrap() = now;
        }

        /// What the device watch does when Windows says drives changed.
        fn device_change(&self) {
            self.devices.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn drive(serial: u32, guid: Option<&str>, mount: &str, label: &str) -> Sighting {
        Sighting {
            volume: Volume {
                id: serial_id(serial),
                label: label.to_owned(),
                mount_path: PathBuf::from(mount),
                kind: volume::VolumeKind::External,
            },
            guid: guid.map(str::to_owned),
        }
    }

    fn serial_id(serial: u32) -> VolumeId {
        volume::identity(volume::IdentitySignals {
            kind: volume::VolumeKind::External,
            unc_share: None,
            serial: Some(serial),
            filesystem: "NTFS",
            guid: None,
        })
        .unwrap()
    }

    fn track_on(id: &VolumeId) -> StoredPath {
        StoredPath::new(id.clone(), RelPath::parse("House/a.mp3").unwrap())
    }

    const GUID_A: &str = "{aaaaaaaa-0000-0000-0000-000000000001}";
    const GUID_B: &str = "{bbbbbbbb-0000-0000-0000-000000000002}";

    #[test]
    fn a_different_drive_on_the_same_letter_never_answers_for_the_old_one_after_a_device_change() {
        let desk = Desk::new(vec![drive(1, Some(GUID_A), r"E:\", "GIG USB")]);
        let volumes = desk.volumes(Vec::new());
        let old = track_on(&serial_id(1));
        assert_eq!(
            old.resolve(&volumes).unwrap(),
            Path::new(r"\\?\E:\House\a.mp3")
        );

        // Unplug it, plug another drive in, and it gets E: too.
        desk.plug(vec![drive(2, Some(GUID_B), r"E:\", "BACKUP")]);
        desk.device_change();
        assert!(matches!(old.resolve(&volumes), Err(PathError::Offline(_))));
        assert_eq!(
            track_on(&serial_id(2)).resolve(&volumes).unwrap(),
            Path::new(r"\\?\E:\House\a.mp3")
        );

        // The old drive comes back on F:, and is found there.
        desk.plug(vec![
            drive(2, Some(GUID_B), r"E:\", "BACKUP"),
            drive(1, Some(GUID_A), r"F:\", "GIG USB"),
        ]);
        desk.device_change();
        assert_eq!(
            old.resolve(&volumes).unwrap(),
            Path::new(r"\\?\F:\House\a.mp3")
        );
    }

    #[test]
    fn the_volumes_are_looked_at_again_only_after_drives_come_or_go() {
        let desk = Desk::new(vec![drive(1, None, r"E:\", "")]);
        let volumes = desk.volumes(Vec::new());
        let track = track_on(&serial_id(1));
        for _ in 0..100 {
            track.resolve(&volumes).unwrap();
        }
        assert_eq!(desk.looks.load(Ordering::SeqCst), 1);
        desk.device_change();
        for _ in 0..100 {
            track.resolve(&volumes).unwrap();
        }
        assert_eq!(desk.looks.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn drives_coming_or_going_make_a_share_be_asked_again_whether_it_answers() {
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        let devices: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        let volumes = SystemVolumes::from_parts(Box::new(Vec::new), devices, Vec::new(), |_| {
            ASKED.fetch_add(1, Ordering::SeqCst);
            true
        });
        let nas = StoredPath::new(share_id(r"\\nas\music"), RelPath::root());
        nas.resolve(&volumes).unwrap();
        nas.resolve(&volumes).unwrap();
        assert_eq!(ASKED.load(Ordering::SeqCst), 1);
        devices.fetch_add(1, Ordering::SeqCst);
        nas.resolve(&volumes).unwrap();
        assert_eq!(ASKED.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn two_clones_plugged_in_together_are_told_apart_by_the_guid_the_library_remembers() {
        let original = drive(1, Some(GUID_A), r"E:\", "GIG USB");
        let backup = drive(1, Some(GUID_B), r"F:\", "GIG USB");
        let desk = Desk::new(vec![backup, original]);
        let remembered = vec![Remembered {
            id: serial_id(1),
            guid: Some(GUID_A.to_owned()),
            label: "GIG USB".to_owned(),
        }];
        let volumes = desk.volumes(remembered);

        // The remembered one keeps the identity; the backup gets its own.
        let shared = serial_id(1);
        let backup_id = shared.with_guid(GUID_B);
        assert_eq!(
            track_on(&shared).resolve(&volumes).unwrap(),
            Path::new(r"\\?\E:\House\a.mp3")
        );
        assert_eq!(
            track_on(&backup_id).resolve(&volumes).unwrap(),
            Path::new(r"\\?\F:\House\a.mp3")
        );
        assert_eq!(volumes.guid(&shared).as_deref(), Some(GUID_A));
        let clashes = volumes.clashes();
        assert_eq!(clashes.len(), 1);
        assert_eq!(clashes[0].kept_by, volume::KeptBy::RememberedGuid);
    }

    #[test]
    fn clones_nothing_tells_apart_both_get_their_own_identity_and_the_shared_one_is_offline() {
        let desk = Desk::new(vec![
            drive(1, Some(GUID_A), r"E:\", "USB"),
            drive(1, Some(GUID_B), r"F:\", "USB"),
        ]);
        let volumes = desk.volumes(Vec::new());
        assert!(matches!(
            track_on(&serial_id(1)).resolve(&volumes),
            Err(PathError::Offline(_))
        ));
        assert!(track_on(&serial_id(1).with_guid(GUID_A))
            .resolve(&volumes)
            .is_ok());
        assert!(track_on(&serial_id(1).with_guid(GUID_B))
            .resolve(&volumes)
            .is_ok());
    }

    fn remembered(id: &VolumeId, guid: &str) -> Remembered {
        Remembered {
            id: id.clone(),
            guid: Some(guid.to_owned()),
            label: "GIG USB".to_owned(),
        }
    }

    #[test]
    fn a_backup_the_library_knows_plugged_in_alone_answers_to_its_own_identity_only() {
        // The backup was told apart once and paths were stored against it;
        // now it's plugged in without the original.
        let desk = Desk::new(vec![drive(1, Some(GUID_B), r"G:\", "GIG USB")]);
        let backup_id = serial_id(1).with_guid(GUID_B);
        let volumes = desk.volumes(vec![
            remembered(&serial_id(1), GUID_A),
            remembered(&backup_id, GUID_B),
        ]);
        assert_eq!(
            track_on(&backup_id).resolve(&volumes).unwrap(),
            Path::new(r"\\?\G:\House\a.mp3")
        );
        // The original's paths show offline, not the backup's files.
        assert!(matches!(
            track_on(&serial_id(1)).resolve(&volumes),
            Err(PathError::Offline(_))
        ));
    }

    #[test]
    fn a_clone_the_library_never_told_apart_plugged_in_alone_is_the_drive() {
        let desk = Desk::new(vec![drive(1, Some(GUID_B), r"G:\", "GIG USB")]);
        let volumes = desk.volumes(vec![remembered(&serial_id(1), GUID_A)]);
        assert!(track_on(&serial_id(1)).resolve(&volumes).is_ok());
    }

    fn at(id: &VolumeId, mount: &str) -> Volume {
        Volume {
            id: id.clone(),
            label: String::new(),
            mount_path: PathBuf::from(mount),
            kind: volume::VolumeKind::External,
        }
    }

    #[test]
    fn a_volume_found_at_a_mount_is_named_the_way_the_last_look_named_it() {
        let backup = serial_id(1).with_guid(GUID_B);
        let mounted = vec![
            (at(&serial_id(1), r"E:\"), Some(GUID_A.to_owned())),
            (at(&backup, r"F:\"), Some(GUID_B.to_owned())),
        ];
        // The plain serial at F: is the backup, named by the clone rule.
        assert_eq!(
            named(at(&serial_id(1), r"F:\"), Some(GUID_B), &mounted, &[])
                .unwrap()
                .id,
            backup
        );
        assert_eq!(
            named(at(&serial_id(1), r"E:\"), Some(GUID_A), &mounted, &[])
                .unwrap()
                .id,
            serial_id(1)
        );
    }

    #[test]
    fn a_volume_the_last_look_did_not_see_there_keeps_the_identity_it_reports() {
        let mounted = vec![(at(&serial_id(1), r"E:\"), None)];
        // Another drive now on E: (arrived since the last look).
        assert_eq!(
            named(at(&serial_id(2), r"E:\"), None, &mounted, &[])
                .unwrap()
                .id,
            serial_id(2)
        );
        // A clone named elsewhere isn't borrowed for a different mount.
        let backup = serial_id(1).with_guid(GUID_B);
        let mounted = vec![(at(&backup, r"F:\"), None)];
        assert_eq!(
            named(at(&serial_id(1), r"G:\"), None, &mounted, &[])
                .unwrap()
                .id,
            serial_id(1)
        );
    }

    #[test]
    fn a_volume_with_another_guid_at_the_same_mount_is_not_named_after_the_last_look() {
        // The drive list says the backup is at F:, but the drive at F: now
        // reports another GUID: the list hasn't caught up with a swap.
        let backup = serial_id(1).with_guid(GUID_B);
        let mounted = vec![(at(&backup, r"F:\"), Some(GUID_B.to_owned()))];
        let now = named(at(&serial_id(1), r"F:\"), Some(GUID_A), &mounted, &[]).unwrap();
        assert_eq!(now.id, serial_id(1));
    }

    #[test]
    fn a_clone_that_cannot_be_told_from_its_twin_cannot_be_named() {
        let clash = CloneClash {
            shared: serial_id(1),
            kept_by: volume::KeptBy::Nobody,
            left_out: vec![PathBuf::from(r"F:\")],
        };
        assert!(named(at(&serial_id(1), r"F:\"), None, &[], &[clash]).is_err());
    }

    #[test]
    fn a_lone_volume_with_the_same_serial_but_another_guid_does_not_answer_for_a_clone() {
        let desk = Desk::new(vec![drive(1, Some(GUID_A), r"E:\", "GIG USB")]);
        let volumes = desk.volumes(Vec::new());
        assert!(matches!(
            track_on(&serial_id(1).with_guid(GUID_B)).resolve(&volumes),
            Err(PathError::Offline(_))
        ));
        // Nor one with another serial and the right GUID.
        let other = Desk::new(vec![drive(2, Some(GUID_B), r"E:\", "GIG USB")]);
        assert!(matches!(
            track_on(&serial_id(1).with_guid(GUID_B)).resolve(&other.volumes(Vec::new())),
            Err(PathError::Offline(_))
        ));
    }

    #[test]
    fn a_clone_with_no_guid_plugged_in_with_its_twin_answers_for_nothing() {
        let desk = Desk::new(vec![
            drive(1, Some(GUID_A), r"E:\", "USB"),
            drive(1, None, r"F:\", "USB"),
        ]);
        let volumes = desk.volumes(Vec::new());
        let mounts: Vec<PathBuf> = volumes
            .mounted()
            .into_iter()
            .map(|v| v.mount_path)
            .collect();
        assert_eq!(mounts, [PathBuf::from(r"E:\")]);
        assert_eq!(volumes.clashes()[0].left_out, [PathBuf::from(r"F:\")]);
    }

    /// A temp dir in its `\\?\` form, so names Win32 would rewrite (like a
    /// trailing dot) can be created in it.
    fn verbatim_tempdir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(dir.path()).unwrap();
        assert!(path.to_str().unwrap().starts_with(r"\\?\"));
        (dir, path)
    }

    fn read_back(stored: &StoredPath, volumes: &SystemVolumes) -> Vec<u8> {
        std::fs::read(stored.resolve(volumes).unwrap()).unwrap()
    }

    #[test]
    fn scan_finds_the_volume_holding_the_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let volumes = SystemVolumes::scan();
        let here = volume::volume_for(dir.path()).unwrap();
        assert!(
            volumes.mounted().iter().any(|v| v.id == here.id),
            "{here:?} not in {:?}",
            volumes.mounted()
        );
    }

    #[test]
    fn a_real_file_round_trips_through_its_stored_form() {
        // Accents, CJK, emoji and `# % + & '`, on a real NTFS disk.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Déjà Nu #1 100% + & 'Live' 坂本 🔥.mp3");
        std::fs::write(&file, b"not really audio").unwrap();

        let volumes = SystemVolumes::scan();
        let stored = StoredPath::from_absolute(&file, &volumes).unwrap();
        assert!(!stored.rel().as_str().contains('\\'));
        let resolved = stored.resolve(&volumes).unwrap();
        assert_eq!(std::fs::read(&resolved).unwrap(), b"not really audio");
        assert_eq!(
            std::fs::canonicalize(&resolved).unwrap(),
            std::fs::canonicalize(&file).unwrap()
        );
    }

    #[test]
    fn a_verbatim_path_to_a_real_file_stores_the_same_as_the_plain_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mp3");
        std::fs::write(&file, b"x").unwrap();
        let verbatim = std::fs::canonicalize(&file).unwrap();
        assert!(verbatim.to_str().unwrap().starts_with(r"\\?\"));

        let volumes = SystemVolumes::scan();
        assert_eq!(
            StoredPath::from_absolute(&verbatim, &volumes).unwrap(),
            StoredPath::from_absolute(&file, &volumes).unwrap()
        );
    }

    #[test]
    fn a_folder_ending_in_a_dot_resolves_to_itself_not_its_dotless_sibling() {
        // Win32 turns `Q.X.Z.\a.mp3` into `Q.X.Z\a.mp3` in a plain path.
        let (_dir, base) = verbatim_tempdir();
        for (folder, bytes) in [("Q.X.Z.", "dotted"), ("Q.X.Z", "plain")] {
            std::fs::create_dir(base.join(folder)).unwrap();
            std::fs::write(base.join(folder).join("a.mp3"), bytes).unwrap();
        }
        let volumes = SystemVolumes::scan();
        let dotted = base.join("Q.X.Z.").join("a.mp3");
        let stored = StoredPath::from_absolute(&dotted, &volumes).unwrap();
        assert!(
            stored.rel().as_str().ends_with("Q.X.Z./a.mp3"),
            "{}",
            stored.rel()
        );
        assert_eq!(read_back(&stored, &volumes), b"dotted");

        // The same path written plainly, as a user might type it.
        let plain_text = dotted.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
        let from_plain = StoredPath::from_absolute(Path::new(plain_text), &volumes).unwrap();
        assert_eq!(from_plain, stored);
        assert_eq!(read_back(&from_plain, &volumes), b"dotted");
    }

    #[test]
    fn names_ending_in_a_dot_or_space_and_device_names_round_trip() {
        let (_dir, base) = verbatim_tempdir();
        let volumes = SystemVolumes::scan();
        for name in ["b.mp3.", "c.mp3 ", "AUX.mp3", "CON"] {
            let file = base.join(name);
            std::fs::write(&file, name).unwrap();
            let stored = StoredPath::from_absolute(&file, &volumes)
                .unwrap_or_else(|e| panic!("storing {name:?}: {e}"));
            assert!(stored.rel().as_str().ends_with(name), "{}", stored.rel());
            assert_eq!(read_back(&stored, &volumes), name.as_bytes(), "{name:?}");
        }
    }

    #[test]
    fn nfd_and_nfc_names_on_ntfs_are_two_files_that_each_resolve_to_themselves() {
        // NTFS doesn't normalize names, which is why the on-disk spelling
        // is what gets stored and resolved.
        let dir = tempfile::tempdir().unwrap();
        let nfd = dir.path().join("Cafe\u{301}.mp3");
        let nfc = dir.path().join("Caf\u{e9}.mp3");
        std::fs::write(&nfd, b"nfd").unwrap();
        std::fs::write(&nfc, b"nfc").unwrap();

        let volumes = SystemVolumes::scan();
        let stored_nfd = StoredPath::from_absolute(&nfd, &volumes).unwrap();
        let stored_nfc = StoredPath::from_absolute(&nfc, &volumes).unwrap();
        assert_ne!(stored_nfd, stored_nfc);
        assert_eq!(stored_nfd.rel().match_key(), stored_nfc.rel().match_key());
        assert_eq!(read_back(&stored_nfd, &volumes), b"nfd");
        assert_eq!(read_back(&stored_nfc, &volumes), b"nfc");
    }

    #[test]
    fn a_file_reached_through_a_junction_is_stored_under_the_junction_target() {
        // `mklink /J` needs no admin rights.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Real Music");
        let link = dir.path().join("Linked Music");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("a.mp3"), b"x").unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink: {made:?}");

        let volumes = SystemVolumes::scan();
        let via_link = StoredPath::from_absolute(&link.join("a.mp3"), &volumes).unwrap();
        let direct = StoredPath::from_absolute(&target.join("a.mp3"), &volumes).unwrap();
        assert_eq!(via_link, direct);
        assert!(
            !via_link.rel().as_str().contains("Linked Music"),
            "{}",
            via_link.rel()
        );
    }

    #[test]
    fn a_missing_file_cannot_be_stored() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.mp3");
        match StoredPath::from_absolute(&missing, &SystemVolumes::scan()) {
            Err(PathError::Unreadable { .. }) => {}
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn a_volume_that_is_not_mounted_is_offline() {
        let volumes = SystemVolumes::scan();
        let dir = tempfile::tempdir().unwrap();
        let stored = StoredPath::from_absolute(dir.path(), &volumes).unwrap();
        // The same path on a volume nobody has plugged in.
        let gone = volume::identity(volume::IdentitySignals {
            kind: volume::VolumeKind::External,
            unc_share: None,
            serial: Some(0xDEAD_BEEF),
            filesystem: "NOT-A-REAL-FS",
            guid: None,
        })
        .unwrap();
        let unplugged = StoredPath::new(gone.clone(), stored.rel().clone());
        for volumes in [&SystemVolumes::with(Vec::new(), |_| true), &volumes] {
            match unplugged.resolve(volumes) {
                Err(PathError::Offline(id)) => assert_eq!(id, gone),
                other => panic!("expected Offline, got {other:?}"),
            }
        }
    }

    fn share_id(unc: &str) -> VolumeId {
        volume::identity(volume::IdentitySignals {
            kind: volume::VolumeKind::Network,
            unc_share: Some(unc),
            serial: None,
            filesystem: "",
            guid: None,
        })
        .unwrap()
    }

    #[test]
    fn a_share_is_asked_whether_it_answers_once_per_scan_not_per_track() {
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        let volumes = SystemVolumes::with(Vec::new(), |_| {
            ASKED.fetch_add(1, Ordering::SeqCst);
            false
        });
        let nas = share_id(r"\\nas\music");
        for track in ["a.mp3", "b.mp3", "House/c.mp3"] {
            let stored = StoredPath::new(nas.clone(), RelPath::parse(track).unwrap());
            assert!(matches!(
                stored.resolve(&volumes),
                Err(PathError::Offline(_))
            ));
        }
        assert_eq!(ASKED.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_slow_share_does_not_hold_up_another_share() {
        static SLOW_ENTERED: AtomicBool = AtomicBool::new(false);
        static RELEASE_SLOW: AtomicBool = AtomicBool::new(false);
        let volumes = SystemVolumes::with(Vec::new(), |root| {
            if root.to_str().unwrap().contains("slow") {
                SLOW_ENTERED.store(true, Ordering::SeqCst);
                while !RELEASE_SLOW.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            true
        });
        let slow = StoredPath::new(share_id(r"\\slow\music"), RelPath::root());
        let fast = StoredPath::new(share_id(r"\\fast\music"), RelPath::root());
        std::thread::scope(|s| {
            let slow_probe = s.spawn(|| slow.resolve(&volumes).unwrap());
            while !SLOW_ENTERED.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            // The slow probe is now running; the fast share must still answer.
            let (done, finished) = mpsc::channel();
            let (fast, volumes) = (&fast, &volumes);
            s.spawn(move || done.send(fast.resolve(volumes).unwrap()).unwrap());
            let fast_result = finished.recv_timeout(Duration::from_secs(5));
            RELEASE_SLOW.store(true, Ordering::SeqCst);
            slow_probe.join().unwrap();
            assert_eq!(
                fast_result.expect("the fast share waited on the slow one"),
                Path::new(r"\\?\UNC\fast\music\")
            );
        });
    }

    #[test]
    fn mount_folders_ending_in_a_dot_are_handed_to_windows_verbatim() {
        assert_eq!(
            verbatim(Path::new(r"C:\mnt\usb.\")).unwrap(),
            Path::new(r"\\?\C:\mnt\usb.")
        );
        assert_eq!(verbatim(Path::new(r"E:\")).unwrap(), Path::new(r"\\?\E:\"));
    }

    #[test]
    fn a_share_that_answers_resolves_to_its_unc_path() {
        let volumes = SystemVolumes::with(Vec::new(), |root| root == Path::new(r"\\nas\music\"));
        let stored = StoredPath::new(
            share_id(r"\\NAS\Music"),
            RelPath::parse("Q.X.Z./a.mp3").unwrap(),
        );
        assert_eq!(
            stored.resolve(&volumes).unwrap(),
            Path::new(r"\\?\UNC\nas\music\Q.X.Z.\a.mp3")
        );
    }

    fn wide_list(names: &[&str]) -> Vec<u16> {
        let mut out: Vec<u16> = Vec::new();
        for name in names {
            out.extend(std::ffi::OsStr::new(name).encode_wide());
            out.push(0);
        }
        out.push(0);
        out
    }

    #[test]
    fn mount_point_lists_that_grow_between_calls_are_read_in_full() {
        // The size query says one mount point; by the real call a second
        // one has appeared and the buffer is too small again.
        let lists = [wide_list(&[r"E:\"]), wide_list(&[r"E:\", r"C:\mnt\usb\"])];
        let mut calls = 0;
        let names = read_multi_sz(|buf, needed| {
            calls += 1;
            let list = if calls == 1 { &lists[0] } else { &lists[1] };
            if buf.len() < list.len() {
                *needed = list.len() as u32;
                return Err(ERROR_MORE_DATA);
            }
            buf[..list.len()].copy_from_slice(list);
            Ok(())
        });
        assert_eq!(
            names,
            [PathBuf::from(r"E:\"), PathBuf::from(r"C:\mnt\usb\")]
        );
        assert_eq!(calls, 3);
    }

    #[test]
    fn a_failing_mount_point_query_gives_no_mount_points() {
        assert!(read_multi_sz(|_, _| Err(5)).is_empty()); // ERROR_ACCESS_DENIED
        assert!(read_multi_sz(|_, needed| {
            *needed = 4;
            Err(ERROR_MORE_DATA)
        })
        .is_empty());
    }
}
