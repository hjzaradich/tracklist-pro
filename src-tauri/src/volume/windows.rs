//! Volume detection on Windows, through the Win32 volume APIs.
//!
//! Only reads: the one handle opened here (to ask a disk which bus it's on)
//! is opened with no read or write access at all.

use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::iter;
use std::mem;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, MAX_PATH};
use windows_sys::Win32::Storage::FileSystem::{
    BusType1394, BusTypeMmc, BusTypeSd, BusTypeUsb, CreateFileW, GetDriveTypeW,
    GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::{classify, identity, Bus, DriveType, Hardware, IdentitySignals, Sighting, Volume};

/// Finds the volume `path` lives on. The path must exist.
///
/// Symlinks, junctions, `subst` drives and mapped network drives are
/// resolved first, so the answer is the volume the data is really on.
pub fn volume_for(path: &Path) -> io::Result<Volume> {
    sighting_for(path).map(|s| s.volume)
}

/// [`volume_for`], plus the volume's GUID, which tells clones apart
/// ([`super::tell_clones_apart`]).
pub fn sighting_for(path: &Path) -> io::Result<Sighting> {
    // Every Win32 call below gets the `\\?\` form that `canonicalize`
    // returns, which Win32 takes as written. A plain path loses trailing
    // dots and spaces (`Q.X.Z.` becomes `Q.X.Z`), and on Windows 10 a name
    // like `AUX.mp3` or `CON` becomes a device.
    let path = std::fs::canonicalize(path)?;
    let verbatim_mount = volume_mount_path(&path)?;
    let root = wide(verbatim_mount.as_os_str());
    let mount_path = without_verbatim_prefix(&verbatim_mount);

    let drive_type = DriveType::from_win32(unsafe { GetDriveTypeW(root.as_ptr()) });
    let info = volume_information(&root);
    let volume_name = volume_name(&root);
    let device = volume_name.as_deref().and_then(storage_device);
    let kind = classify(Hardware {
        drive_type,
        bus: device.map(|d| d.bus),
        removable_media: device.is_some_and(|d| d.removable_media),
    });

    let unc_share = unc_share(&mount_path);
    let guid = volume_name.as_deref().and_then(guid_of);
    let id = identity(IdentitySignals {
        kind,
        unc_share: unc_share.as_deref(),
        serial: info.as_ref().map(|i| i.serial),
        filesystem: info.as_ref().map_or("", |i| &i.filesystem),
        guid,
    })
    .ok_or_else(|| {
        io::Error::other(format!(
            "the volume at {} has no serial number or GUID",
            mount_path.display()
        ))
    })?;

    Ok(Sighting {
        volume: Volume {
            id,
            label: info.map(|i| i.label).unwrap_or_default(),
            mount_path,
            kind,
        },
        guid: guid.map(str::to_lowercase),
    })
}

/// Turns `\\?\C:\x` into `C:\x` and `\\?\UNC\server\share\x` into
/// `\\server\share\x`. Other paths are returned unchanged.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let mut root = OsString::new();
    match prefix.kind() {
        Prefix::VerbatimDisk(letter) => root.push(format!("{}:\\", letter as char)),
        Prefix::VerbatimUNC(server, share) => {
            root.push(r"\\");
            root.push(server);
            root.push(r"\");
            root.push(share);
            root.push(r"\");
        }
        _ => return path.to_path_buf(),
    }
    let mut out = PathBuf::from(root);
    out.extend(components.filter(|c| !matches!(c, Component::RootDir)));
    out
}

/// `\\server\share` for a path on a network share.
fn unc_share(path: &Path) -> Option<String> {
    match path.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(format!(
                r"\\{}\{}",
                server.to_string_lossy(),
                share.to_string_lossy()
            )),
            _ => None,
        },
        _ => None,
    }
}

/// The mount point of the volume holding `path`, with a trailing
/// backslash: `C:\`, `C:\mnt\usb\` or `\\server\share\`. A `\\?\` path
/// gives a `\\?\` answer (`\\?\C:\`, `\\?\UNC\server\share\`).
fn volume_mount_path(path: &Path) -> io::Result<PathBuf> {
    let input = wide(path.as_os_str());
    // The mount point is never longer than the path itself.
    let mut buf = vec![0u16; input.len().max(MAX_PATH as usize) + 1];
    let ok = unsafe { GetVolumePathNameW(input.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(from_wide(&buf)))
}

struct VolumeInformation {
    label: String,
    serial: u32,
    filesystem: String,
}

/// Label, serial number and filesystem name, or `None` if the volume
/// won't say (e.g. a network share that denies it).
fn volume_information(root: &[u16]) -> Option<VolumeInformation> {
    let mut label = [0u16; MAX_PATH as usize + 1];
    let mut filesystem = [0u16; MAX_PATH as usize + 1];
    let mut serial = 0u32;
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            ptr::null_mut(),
            ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    };
    (ok != 0).then(|| VolumeInformation {
        label: from_wide(&label).to_string_lossy().into_owned(),
        serial,
        filesystem: from_wide(&filesystem).to_string_lossy().into_owned(),
    })
}

/// The volume's `\\?\Volume{GUID}\` name. Local volumes only.
fn volume_name(root: &[u16]) -> Option<String> {
    let mut buf = [0u16; 64];
    let ok = unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), buf.as_mut_ptr(), buf.len() as u32)
    };
    (ok != 0).then(|| from_wide(&buf).to_string_lossy().into_owned())
}

/// `{GUID}` out of `\\?\Volume{GUID}\`.
fn guid_of(volume_name: &str) -> Option<&str> {
    let start = volume_name.find('{')?;
    let end = volume_name[start..].find('}')? + start;
    Some(&volume_name[start..=end])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StorageDevice {
    bus: Bus,
    removable_media: bool,
}

/// Asks the disk behind a volume which bus it's on. `None` if it can't be
/// asked, e.g. for a volume spanning several disks.
fn storage_device(volume_name: &str) -> Option<StorageDevice> {
    // `\\?\Volume{GUID}` without the trailing backslash opens the volume
    // device rather than its root folder.
    let device = wide(OsStr::new(volume_name.trim_end_matches('\\')));
    // Access 0: query only. No read or write access to the volume.
    let handle = unsafe {
        CreateFileW(
            device.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let _close = CloseOnDrop(handle);

    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // 1 KiB: room for the descriptor and its strings.
    let mut out = [0u8; 1024];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&query as *const STORAGE_PROPERTY_QUERY).cast::<c_void>(),
            mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            out.as_mut_ptr().cast::<c_void>(),
            out.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    parse_device_descriptor(out.get(..returned as usize)?)
}

// Where the fields we need sit in a STORAGE_DEVICE_DESCRIPTOR. `offset_of!`
// only computes the position; no descriptor value is ever built.
const REMOVABLE_MEDIA_OFFSET: usize = mem::offset_of!(STORAGE_DEVICE_DESCRIPTOR, RemovableMedia);
const BUS_TYPE_OFFSET: usize = mem::offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType);

/// Reads the bus and removable flag out of the raw descriptor bytes the
/// driver returned. windows-sys declares `RemovableMedia` as a Rust `bool`,
/// and reading a driver's byte (which may be any value, e.g. 0xFF) into a
/// `bool` is undefined behavior, so the bytes are read directly: any
/// non-zero byte means removable. `None` if the reply is too short.
fn parse_device_descriptor(bytes: &[u8]) -> Option<StorageDevice> {
    let removable = *bytes.get(REMOVABLE_MEDIA_OFFSET)?;
    let bus_type = bytes.get(BUS_TYPE_OFFSET..BUS_TYPE_OFFSET + 4)?;
    Some(StorageDevice {
        bus: bus_from_win32(i32::from_ne_bytes(bus_type.try_into().ok()?)),
        removable_media: removable != 0,
    })
}

// The bus-type constants keep their Win32 names.
#[allow(non_upper_case_globals)]
fn bus_from_win32(bus_type: i32) -> Bus {
    match bus_type {
        BusTypeUsb => Bus::Usb,
        BusType1394 => Bus::Firewire,
        BusTypeSd | BusTypeMmc => Bus::SdCard,
        _ => Bus::Other,
    }
}

struct CloseOnDrop(HANDLE);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// A NUL-terminated UTF-16 string for a Win32 call.
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(iter::once(0)).collect()
}

/// The UTF-16 string in `buf`, up to its first NUL.
fn from_wide(buf: &[u16]) -> OsString {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::VolumeKind;

    fn drive_letter_of(path: &Path) -> Option<char> {
        match without_verbatim_prefix(path).components().next()? {
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(letter) => Some((letter as char).to_ascii_uppercase()),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn the_temp_dir_resolves_to_a_local_volume() {
        let dir = tempfile::tempdir().unwrap();
        let volume = volume_for(dir.path()).unwrap();
        assert!(!volume.id.as_str().is_empty());
        assert_ne!(volume.kind, VolumeKind::Network, "{volume:?}");
        let canonical = without_verbatim_prefix(&std::fs::canonicalize(dir.path()).unwrap());
        assert!(
            canonical.starts_with(&volume.mount_path),
            "{canonical:?} is not under {volume:?}"
        );
    }

    #[test]
    fn volume_identity_never_contains_the_drive_letter() {
        let dir = tempfile::tempdir().unwrap();
        let volume = volume_for(dir.path()).unwrap();
        let letter = drive_letter_of(&volume.mount_path).expect("temp dir is on a lettered drive");
        let id = volume.id.as_str().to_uppercase();
        assert!(
            !id.contains(&format!("{letter}:")),
            "{} contains {letter}:",
            volume.id
        );
        assert!(!id.contains(':'), "{}", volume.id);
    }

    #[test]
    fn the_temp_dir_volume_is_identified_by_serial_or_guid() {
        let dir = tempfile::tempdir().unwrap();
        let id = volume_for(dir.path()).unwrap().id;
        assert!(
            id.as_str().starts_with("serial=") || id.as_str().starts_with("guid="),
            "{id}"
        );
    }

    #[test]
    fn every_path_on_one_volume_gives_the_same_volume() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("track.txt");
        std::fs::write(&file, b"x").unwrap();
        let expected = volume_for(dir.path()).unwrap();
        assert_eq!(volume_for(&nested).unwrap(), expected);
        assert_eq!(volume_for(&file).unwrap(), expected);
        assert_eq!(volume_for(&expected.mount_path).unwrap(), expected);
    }

    #[test]
    fn a_verbatim_path_gives_the_same_volume_as_the_plain_path() {
        let dir = tempfile::tempdir().unwrap();
        let plain = without_verbatim_prefix(&std::fs::canonicalize(dir.path()).unwrap());
        let verbatim = PathBuf::from(format!(r"\\?\{}", plain.display()));
        assert_eq!(volume_for(&verbatim).unwrap(), volume_for(&plain).unwrap());
    }

    #[test]
    fn device_names_and_trailing_dots_in_a_path_still_give_its_volume() {
        // Plain Win32 paths drop trailing dots and, on Windows 10 and
        // Server 2022, turn `AUX.mp3` or `CON` into devices; the `\\?\`
        // form keeps them files.
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let expected = volume_for(&base).unwrap();
        std::fs::create_dir(base.join("Q.X.Z.")).unwrap();
        for name in ["AUX.mp3", "CON", "b.mp3.", r"Q.X.Z.\a.mp3"] {
            let file = base.join(name);
            std::fs::write(&file, b"x").unwrap();
            let got = volume_for(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    fn a_lettered_drive_is_mounted_at_its_root() {
        let dir = tempfile::tempdir().unwrap();
        let volume = volume_for(dir.path()).unwrap();
        let letter = drive_letter_of(&volume.mount_path).unwrap();
        assert_eq!(
            volume.mount_path.to_string_lossy().to_uppercase(),
            format!(r"{letter}:\")
        );
    }

    #[test]
    fn the_temp_dir_disk_answers_the_bus_query() {
        // If this query failed everywhere, USB drives would silently be
        // classed as internal.
        let dir = tempfile::tempdir().unwrap();
        let mount_path = volume_for(dir.path()).unwrap().mount_path;
        let name =
            volume_name(&wide(mount_path.as_os_str())).expect("local volume has a GUID name");
        assert!(
            storage_device(&name).is_some(),
            "bus query failed for {name}"
        );
    }

    #[test]
    fn a_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = volume_for(&dir.path().join("not-there")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn verbatim_prefixes_are_removed() {
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\C:\Music\a.mp3")),
            PathBuf::from(r"C:\Music\a.mp3")
        );
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\C:\")),
            PathBuf::from(r"C:\")
        );
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\UNC\nas\music\house\a.mp3")),
            PathBuf::from(r"\\nas\music\house\a.mp3")
        );
        assert_eq!(
            without_verbatim_prefix(Path::new(r"D:\Music")),
            PathBuf::from(r"D:\Music")
        );
    }

    #[test]
    fn unc_share_is_found_for_network_paths_only() {
        assert_eq!(
            unc_share(Path::new(r"\\nas\music\")).as_deref(),
            Some(r"\\nas\music")
        );
        assert_eq!(
            unc_share(Path::new(r"\\?\UNC\nas\music\x")).as_deref(),
            Some(r"\\nas\music")
        );
        assert_eq!(unc_share(Path::new(r"C:\")), None);
    }

    #[test]
    fn guid_is_taken_from_the_volume_name() {
        assert_eq!(
            guid_of(r"\\?\Volume{0a1b2c3d-0000-1111-2222-333344445555}\"),
            Some("{0a1b2c3d-0000-1111-2222-333344445555}")
        );
        assert_eq!(guid_of(r"C:\"), None);
    }

    /// A descriptor as a driver might return it: every byte 0xFF except the
    /// bus type, so the bool-typed fields hold values a Rust `bool` can't.
    fn descriptor_bytes(bus_type: i32) -> Vec<u8> {
        let mut bytes = vec![0xFF; mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>()];
        bytes[BUS_TYPE_OFFSET..BUS_TYPE_OFFSET + 4].copy_from_slice(&bus_type.to_ne_bytes());
        bytes
    }

    #[test]
    fn descriptor_field_offsets_match_the_documented_windows_layout() {
        // Version u32, Size u32, DeviceType u8, DeviceTypeModifier u8,
        // RemovableMedia at 10; four u32 offsets, then BusType at 28.
        assert_eq!(REMOVABLE_MEDIA_OFFSET, 10);
        assert_eq!(BUS_TYPE_OFFSET, 28);
    }

    #[test]
    fn a_descriptor_with_0xff_in_its_bool_fields_parses_as_removable() {
        let device = parse_device_descriptor(&descriptor_bytes(BusTypeUsb)).unwrap();
        assert_eq!(
            device,
            StorageDevice {
                bus: Bus::Usb,
                removable_media: true
            }
        );
    }

    #[test]
    fn a_descriptor_with_a_zero_removable_byte_parses_as_fixed_media() {
        let mut bytes = descriptor_bytes(17); // NVMe
        bytes[REMOVABLE_MEDIA_OFFSET] = 0;
        let device = parse_device_descriptor(&bytes).unwrap();
        assert_eq!(
            device,
            StorageDevice {
                bus: Bus::Other,
                removable_media: false
            }
        );
    }

    #[test]
    fn a_descriptor_too_short_to_hold_the_bus_type_is_rejected() {
        let bytes = descriptor_bytes(BusTypeUsb);
        assert_eq!(parse_device_descriptor(&bytes[..BUS_TYPE_OFFSET + 3]), None);
        assert_eq!(
            parse_device_descriptor(&bytes[..REMOVABLE_MEDIA_OFFSET]),
            None
        );
        assert_eq!(parse_device_descriptor(&[]), None);
    }

    #[test]
    fn win32_bus_types_map_to_buses() {
        assert_eq!(bus_from_win32(BusTypeUsb), Bus::Usb);
        assert_eq!(bus_from_win32(BusType1394), Bus::Firewire);
        assert_eq!(bus_from_win32(BusTypeSd), Bus::SdCard);
        assert_eq!(bus_from_win32(BusTypeMmc), Bus::SdCard);
        // SATA (11) and NVMe (17) are built in.
        assert_eq!(bus_from_win32(11), Bus::Other);
        assert_eq!(bus_from_win32(17), Bus::Other);
    }
}
