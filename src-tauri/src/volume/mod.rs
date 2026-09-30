//! Volume detection (ROADMAP §0.3): which disk or share a path lives on,
//! identified in a way that survives drive-letter changes.
//!
//! A stored path is a volume identity plus a path relative to the volume's
//! mount point, so a drive that comes back as `F:` instead of `E:` still
//! resolves. The `volume` table stores each identity as text; read it back
//! with [`VolumeId::from_stored`].
//!
//! Identity is serial-first, and a sector clone of a drive (a backup made
//! with a cloning tool) has the same serial. [`tell_clones_apart`] is the
//! rule for when both are plugged in at once.

// The classification and identity rules are Windows logic, but they're
// platform-neutral code so their tests run everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

#[cfg(windows)]
pub mod devices;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::{sighting_for, volume_for};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use self::unix::volume_for;

/// A stable identity for a volume. It never contains a drive letter or a
/// mount path, so it doesn't change when the drive is mounted elsewhere.
///
/// On Windows, in order of preference:
/// - `unc=\\server\share` for a network share, lowercased.
/// - `serial=NTFS-1A2B3C4D`: the filesystem's serial number. It's stored on
///   the disk itself, so it is the same on every PC and survives a Windows
///   reinstall (needed for the portability bundle, ROADMAP 4.1). It changes
///   only when the volume is reformatted.
/// - `guid={...}`: the volume GUID, for a volume that reports no serial.
///   Windows assigns it per PC, so it's only the fallback.
/// - `serial=NTFS-1A2B3C4D+guid={...}`: a drive whose serial is shared with
///   a clone plugged in at the same time, told apart by its GUID
///   ([`tell_clones_apart`]).
///
/// (`dev=…` is the unsupported macOS/Linux fallback.)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VolumeId(String);

impl VolumeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Checks an identity read back from the database. Accepts exactly the
    /// forms this module builds, so a drive letter, a mount path or a
    /// hand-edited value is refused rather than trusted.
    pub fn from_stored(stored: String) -> Result<VolumeId, BadVolumeId> {
        if valid_identity(&stored) {
            Ok(VolumeId(stored))
        } else {
            Err(BadVolumeId(stored))
        }
    }

    /// A plain serial identity, the only kind two volumes can share.
    pub fn is_plain_serial(&self) -> bool {
        self.0.starts_with("serial=") && !self.0.contains("+guid=")
    }

    /// This serial identity narrowed to the volume with `guid`.
    pub fn with_guid(&self, guid: &str) -> VolumeId {
        VolumeId(format!("{}+guid={}", self.0, guid.to_lowercase()))
    }

    /// For a clone's identity, `serial=…+guid={…}`: the plain serial
    /// identity it shares and its own GUID. `None` for any other form.
    pub fn clone_parts(&self) -> Option<(VolumeId, &str)> {
        let (serial, guid) = self.0.split_once("+guid=")?;
        serial
            .starts_with("serial=")
            .then(|| (VolumeId(serial.to_owned()), guid))
    }
}

impl fmt::Display for VolumeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A stored volume identity that isn't one this app builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadVolumeId(pub String);

impl fmt::Display for BadVolumeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} isn't a volume identity", self.0)
    }
}

impl std::error::Error for BadVolumeId {}

/// Whether `s` is a form [`identity`] or [`tell_clones_apart`] builds.
fn valid_identity(s: &str) -> bool {
    // A colon would mean a drive letter (`E:`); no form has one.
    if s.contains(':') || s.chars().any(char::is_control) {
        return false;
    }
    if let Some(share) = s.strip_prefix("unc=") {
        return valid_unc(share);
    }
    if let Some(guid) = s.strip_prefix("guid=") {
        return valid_guid(guid);
    }
    if let Some(dev) = s.strip_prefix("dev=") {
        return !dev.is_empty() && dev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    }
    if let Some(rest) = s.strip_prefix("serial=") {
        return match rest.split_once("+guid=") {
            Some((serial, guid)) => valid_serial(serial) && valid_guid(guid),
            None => valid_serial(rest),
        };
    }
    false
}

/// `\\server\share`, lowercased, with nothing after the share.
fn valid_unc(share: &str) -> bool {
    let Some(rest) = share.strip_prefix(r"\\") else {
        return false;
    };
    let mut parts = rest.split('\\');
    let (Some(server), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !server.is_empty() && !name.is_empty() && share == share.to_lowercase()
}

/// `FILESYSTEM-1A2B3C4D`: the filesystem name in uppercase letters and
/// digits (possibly empty) and a non-zero serial as eight uppercase hex
/// digits.
fn valid_serial(serial: &str) -> bool {
    let Some((filesystem, hex)) = serial.split_once('-') else {
        return false;
    };
    hex.len() == 8
        && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'F'))
        && hex != "00000000"
        && filesystem
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `{…}`: lowercase hex digits and dashes in braces.
fn valid_guid(guid: &str) -> bool {
    let Some(inner) = guid.strip_prefix('{').and_then(|g| g.strip_suffix('}')) else {
        return false;
    };
    !inner.is_empty()
        && inner
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'-'))
}

/// Where a volume is attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// A disk inside the computer.
    Internal,
    /// A disk that can be unplugged: USB drives, SD cards, optical discs.
    External,
    /// A network share.
    Network,
}

/// The volume a path lives on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub id: VolumeId,
    /// The volume's label as the OS reports it. May be empty.
    pub label: String,
    /// Where the volume is mounted right now, e.g. `E:\` or
    /// `\\server\share\`. This is what changes between sessions.
    pub mount_path: PathBuf,
    pub kind: VolumeKind,
}

/// The drive type Windows reports for a volume root (`GetDriveTypeW`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriveType {
    Unknown,
    Removable,
    Fixed,
    Remote,
    CdRom,
    RamDisk,
}

impl DriveType {
    /// Maps a `GetDriveTypeW` return value.
    pub(crate) fn from_win32(value: u32) -> DriveType {
        match value {
            2 => DriveType::Removable,
            3 => DriveType::Fixed,
            4 => DriveType::Remote,
            5 => DriveType::CdRom,
            6 => DriveType::RamDisk,
            // 0 = unknown, 1 = no such root.
            _ => DriveType::Unknown,
        }
    }
}

/// The bus a disk is attached through, as far as it matters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bus {
    Usb,
    Firewire,
    SdCard,
    /// SATA, NVMe, SCSI, virtual disks and anything else built in.
    Other,
}

/// What the OS says about a volume's hardware, as input to [`classify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hardware {
    pub drive_type: DriveType,
    /// `None` when the disk couldn't be asked (e.g. a volume spanning disks).
    pub bus: Option<Bus>,
    pub removable_media: bool,
}

/// Decides whether a volume is internal, external or a network share.
///
/// USB hard drives and SSDs report themselves as fixed disks, so the drive
/// type alone isn't enough: the bus decides.
pub(crate) fn classify(hw: Hardware) -> VolumeKind {
    match hw.drive_type {
        DriveType::Remote => VolumeKind::Network,
        DriveType::Removable | DriveType::CdRom => VolumeKind::External,
        DriveType::Fixed | DriveType::RamDisk | DriveType::Unknown => {
            let external_bus = matches!(hw.bus, Some(Bus::Usb | Bus::Firewire | Bus::SdCard));
            if external_bus || hw.removable_media {
                VolumeKind::External
            } else {
                VolumeKind::Internal
            }
        }
    }
}

/// What a volume reports about itself, as input to [`identity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdentitySignals<'a> {
    pub kind: VolumeKind,
    /// `\\server\share` for a network volume.
    pub unc_share: Option<&'a str>,
    /// The filesystem's serial number. Some volumes report 0, meaning none.
    pub serial: Option<u32>,
    /// The filesystem name, e.g. `NTFS` or `exFAT`.
    pub filesystem: &'a str,
    /// The volume GUID, e.g. `{0a1b...}`.
    pub guid: Option<&'a str>,
}

/// Builds the [`VolumeId`] for a volume, or `None` if it has nothing stable
/// to be identified by.
pub(crate) fn identity(s: IdentitySignals<'_>) -> Option<VolumeId> {
    if s.kind == VolumeKind::Network {
        let share = s.unc_share?.trim_end_matches('\\');
        return Some(VolumeId(format!("unc={}", share.to_lowercase())));
    }
    if let Some(serial) = s.serial.filter(|&n| n != 0) {
        // Letters and digits only, so the name can't carry the separators
        // of the identity forms (`-`, `+`, `=`): `HFS+` becomes `HFS`.
        let filesystem: String = s
            .filesystem
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .collect();
        return Some(VolumeId(format!("serial={filesystem}-{serial:08X}")));
    }
    s.guid
        .map(|guid| VolumeId(format!("guid={}", guid.to_lowercase())))
}

/// One mount point a volume scan found: the volume as [`identity`] named it,
/// plus its volume GUID, which is what tells clones apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    pub volume: Volume,
    /// The volume GUID, e.g. `{0a1b…}`. `None` if Windows didn't give one.
    pub guid: Option<String>,
}

/// What the library remembers about a volume from the last time it was
/// seen: the `volume` table's `guid` and `label`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub id: VolumeId,
    pub guid: Option<String>,
    pub label: String,
}

/// How the volume that keeps a shared serial identity was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptBy {
    /// Its GUID is the one the library remembers for that identity.
    RememberedGuid,
    /// It's the only one whose label is the one the library remembers.
    RememberedLabel,
    /// Nothing told them apart, so none keeps it. Paths stored against the
    /// shared identity resolve as offline until only one is plugged in (or
    /// the user says which is which).
    Nobody,
}

/// Two or more mounted volumes that share one serial identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneClash {
    /// The identity they share.
    pub shared: VolumeId,
    pub kept_by: KeptBy,
    /// Mount points of clones with no GUID. They can't be told apart from
    /// the others, so they're left out of the scan (they resolve as
    /// offline) rather than guessed at.
    pub left_out: Vec<PathBuf>,
}

/// The mounted volumes after [`tell_clones_apart`], sorted by identity and
/// then mount path, and any clashes it found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    pub volumes: Vec<Volume>,
    pub clashes: Vec<CloneClash>,
}

/// The clone rule. Identity is serial-first, so a sector clone of a drive
/// has the same identity as the drive. Plugged in alone, a clone the
/// library has never told apart *is* the drive as far as stored paths go,
/// which is what a backup is for. Plugged in together, they must not both
/// answer to one identity, or files on the backup would be taken for files
/// on the original.
///
/// Among the sightings that share a plain `serial=` identity:
/// 1. Sightings with the same GUID are one volume at several mount points,
///    not clones. Network shares (`unc=`) and `guid=` identities never
///    clash: the share path and the GUID are unique already.
/// 2. With two or more GUIDs, the volume that keeps the plain identity is
///    the one whose GUID the library remembers for it; failing that, the
///    only one whose label the library remembers; failing that, none.
/// 3. Every other one gets `serial=…+guid={…}`, its own GUID added, so it
///    has a stable identity of its own on this PC.
/// 4. A sighting with no GUID in a clash can't be told apart, so it's left
///    out (offline) and listed in [`CloneClash::left_out`].
/// 5. A clone the library already knows by its own identity
///    (`serial=…+guid={…}` is remembered) keeps that identity, even when
///    it's plugged in alone, and never keeps the shared one in a clash
///    (neither by GUID nor by label). The original's paths then show
///    offline rather than resolving to the backup.
///
/// The result doesn't depend on the order the scan found the volumes.
pub fn tell_clones_apart(sightings: Vec<Sighting>, remembered: &[Remembered]) -> Scan {
    let mut groups: BTreeMap<VolumeId, Vec<Sighting>> = BTreeMap::new();
    for s in sightings {
        groups.entry(s.volume.id.clone()).or_default().push(s);
    }
    let mut scan = Scan::default();
    for (shared, group) in groups {
        let guids: BTreeSet<String> = group
            .iter()
            .filter_map(|s| s.guid.as_deref().map(str::to_lowercase))
            .collect();
        let has_guidless = group.iter().any(|s| s.guid.is_none());
        let physical = guids.len() + usize::from(has_guidless);
        if !shared.is_plain_serial() || physical <= 1 {
            for s in group {
                let own = s
                    .guid
                    .as_deref()
                    .filter(|_| shared.is_plain_serial())
                    .map(|g| shared.with_guid(g))
                    .filter(|own| remembered.iter().any(|r| &r.id == own));
                scan.volumes.push(match own {
                    Some(id) => Volume { id, ..s.volume },
                    None => s.volume,
                });
            }
            continue;
        }

        let memory = remembered.iter().find(|r| r.id == shared);
        // A clone the library knows by its own identity (rule 5) can never
        // keep the shared one, whatever its GUID or label.
        let has_own_identity = |g: &str| remembered.iter().any(|r| r.id == shared.with_guid(g));
        let by_guid = memory
            .and_then(|r| r.guid.as_deref())
            .map(str::to_lowercase)
            .filter(|g| guids.contains(g) && !has_own_identity(g));
        let by_label = || {
            let label = &memory?.label;
            let matching: BTreeSet<String> = group
                .iter()
                .filter(|s| &s.volume.label == label)
                .filter_map(|s| s.guid.as_deref().map(str::to_lowercase))
                .filter(|g| !has_own_identity(g))
                .collect();
            (matching.len() == 1)
                .then(|| matching.into_iter().next())
                .flatten()
        };
        let (keeper, kept_by) = match by_guid {
            Some(g) => (Some(g), KeptBy::RememberedGuid),
            None => match by_label() {
                Some(g) => (Some(g), KeptBy::RememberedLabel),
                None => (None, KeptBy::Nobody),
            },
        };

        let mut left_out = Vec::new();
        for s in group {
            let Some(guid) = s.guid.as_deref().map(str::to_lowercase) else {
                left_out.push(s.volume.mount_path);
                continue;
            };
            let id = if keeper.as_deref() == Some(guid.as_str()) {
                shared.clone()
            } else {
                shared.with_guid(&guid)
            };
            scan.volumes.push(Volume { id, ..s.volume });
        }
        left_out.sort();
        scan.clashes.push(CloneClash {
            shared,
            kept_by,
            left_out,
        });
    }
    scan.volumes
        .sort_by(|a, b| (&a.id, &a.mount_path, &a.label).cmp(&(&b.id, &b.mount_path, &b.label)));
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn hw(drive_type: DriveType, bus: Option<Bus>, removable_media: bool) -> Hardware {
        Hardware {
            drive_type,
            bus,
            removable_media,
        }
    }

    #[test]
    fn remote_drives_are_network() {
        assert_eq!(
            classify(hw(DriveType::Remote, None, false)),
            VolumeKind::Network
        );
    }

    #[test]
    fn removable_drives_are_external() {
        // USB sticks and SD card readers.
        assert_eq!(
            classify(hw(DriveType::Removable, Some(Bus::Usb), true)),
            VolumeKind::External
        );
        assert_eq!(
            classify(hw(DriveType::Removable, None, false)),
            VolumeKind::External
        );
    }

    #[test]
    fn optical_drives_are_external() {
        assert_eq!(
            classify(hw(DriveType::CdRom, None, true)),
            VolumeKind::External
        );
    }

    #[test]
    fn fixed_disks_on_usb_firewire_or_sd_are_external() {
        // USB hard drives and SSDs report themselves as fixed disks.
        for bus in [Bus::Usb, Bus::Firewire, Bus::SdCard] {
            assert_eq!(
                classify(hw(DriveType::Fixed, Some(bus), false)),
                VolumeKind::External,
                "{bus:?}"
            );
        }
    }

    #[test]
    fn fixed_disks_with_removable_media_are_external() {
        assert_eq!(
            classify(hw(DriveType::Fixed, Some(Bus::Other), true)),
            VolumeKind::External
        );
    }

    #[test]
    fn fixed_disks_on_internal_or_unknown_buses_are_internal() {
        assert_eq!(
            classify(hw(DriveType::Fixed, Some(Bus::Other), false)),
            VolumeKind::Internal
        );
        assert_eq!(
            classify(hw(DriveType::Fixed, None, false)),
            VolumeKind::Internal
        );
        assert_eq!(
            classify(hw(DriveType::RamDisk, None, false)),
            VolumeKind::Internal
        );
        assert_eq!(
            classify(hw(DriveType::Unknown, None, false)),
            VolumeKind::Internal
        );
    }

    #[test]
    fn win32_drive_type_values_map_to_drive_types() {
        assert_eq!(DriveType::from_win32(0), DriveType::Unknown);
        assert_eq!(DriveType::from_win32(1), DriveType::Unknown);
        assert_eq!(DriveType::from_win32(2), DriveType::Removable);
        assert_eq!(DriveType::from_win32(3), DriveType::Fixed);
        assert_eq!(DriveType::from_win32(4), DriveType::Remote);
        assert_eq!(DriveType::from_win32(5), DriveType::CdRom);
        assert_eq!(DriveType::from_win32(6), DriveType::RamDisk);
    }

    fn local(serial: Option<u32>, guid: Option<&str>) -> IdentitySignals<'_> {
        IdentitySignals {
            kind: VolumeKind::External,
            unc_share: None,
            serial,
            filesystem: "exFAT",
            guid,
        }
    }

    #[test]
    fn local_identity_is_the_filesystem_serial() {
        let id = identity(local(Some(0x1A2B_3C4D), Some("{AAAA-BBBB}"))).unwrap();
        assert_eq!(id.as_str(), "serial=EXFAT-1A2B3C4D");
    }

    #[test]
    fn serial_identity_is_zero_padded_to_eight_hex_digits() {
        let id = identity(local(Some(0xBEEF), None)).unwrap();
        assert_eq!(id.as_str(), "serial=EXFAT-0000BEEF");
    }

    #[test]
    fn local_identity_falls_back_to_the_volume_guid_without_a_serial() {
        for serial in [None, Some(0)] {
            let id = identity(local(serial, Some("{0A1B-C2D3}"))).unwrap();
            assert_eq!(id.as_str(), "guid={0a1b-c2d3}");
        }
    }

    #[test]
    fn a_volume_with_no_serial_and_no_guid_has_no_identity() {
        assert_eq!(identity(local(None, None)), None);
    }

    #[test]
    fn network_identity_is_the_lowercased_share_not_a_drive_letter() {
        let signals = IdentitySignals {
            kind: VolumeKind::Network,
            unc_share: Some(r"\\NAS\Music\"),
            serial: Some(0x1234),
            filesystem: "NTFS",
            guid: None,
        };
        assert_eq!(identity(signals).unwrap().as_str(), r"unc=\\nas\music");
    }

    #[test]
    fn no_identity_form_can_contain_a_drive_letter() {
        // A drive letter needs a colon (`E:`); no identity form has one.
        let network = IdentitySignals {
            kind: VolumeKind::Network,
            unc_share: Some(r"\\nas\music"),
            serial: None,
            filesystem: "",
            guid: None,
        };
        let ids = [
            identity(local(Some(0xCAFE_F00D), None)).unwrap(),
            identity(local(None, Some("{0A1B-C2D3}"))).unwrap(),
            identity(network).unwrap(),
        ];
        for id in ids {
            assert!(!id.as_str().contains(':'), "{id}");
        }
    }

    // Identities read back from the database.

    fn network(unc: &str) -> VolumeId {
        identity(IdentitySignals {
            kind: VolumeKind::Network,
            unc_share: Some(unc),
            serial: None,
            filesystem: "",
            guid: None,
        })
        .unwrap()
    }

    #[test]
    fn every_identity_form_the_app_builds_is_accepted_back_from_the_database() {
        let serial = identity(local(Some(0x1A2B_3C4D), None)).unwrap();
        let built = [
            serial.clone(),
            identity(local(Some(0xBEEF), None)).unwrap(),
            identity(local(None, Some("{0A1B-C2D3}"))).unwrap(),
            network(r"\\NAS\Music\"),
            serial.with_guid("{0A1B2C3D-0000-1111-2222-333344445555}"),
            // Filesystem names with separators in them, and an empty one.
            identity(IdentitySignals {
                filesystem: "NOT-A-REAL-FS",
                ..local(Some(1), None)
            })
            .unwrap(),
            identity(IdentitySignals {
                filesystem: "HFS+",
                ..local(Some(1), None)
            })
            .unwrap(),
            identity(IdentitySignals {
                filesystem: "a=b+c",
                ..local(Some(1), None)
            })
            .unwrap(),
            identity(IdentitySignals {
                filesystem: "",
                ..local(Some(1), None)
            })
            .unwrap(),
            VolumeId("dev=803".into()),
        ];
        for id in built {
            assert_eq!(VolumeId::from_stored(id.to_string()), Ok(id.clone()));
        }
    }

    #[test]
    fn a_filesystem_name_keeps_only_its_letters_and_digits_in_the_identity() {
        for (filesystem, expected) in [
            ("HFS+", "serial=HFS-1A2B3C4D"),
            ("exFAT", "serial=EXFAT-1A2B3C4D"),
            ("FAT32", "serial=FAT32-1A2B3C4D"),
            ("NOT-A-REAL-FS", "serial=NOTAREALFS-1A2B3C4D"),
            ("", "serial=-1A2B3C4D"),
        ] {
            let id = identity(IdentitySignals {
                filesystem,
                ..local(Some(0x1A2B_3C4D), None)
            })
            .unwrap();
            assert_eq!(id.as_str(), expected, "{filesystem:?}");
        }
    }

    #[test]
    fn a_stored_identity_the_app_never_builds_is_refused() {
        for bad in [
            "",
            "E:",
            r"E:\",
            r"E:\Music",
            r"\\nas\music",
            "serial=",
            "serial=NTFS",
            "serial=NTFS-1A2B3C4",        // seven digits
            "serial=NTFS-1a2b3c4d",       // lowercase hex
            "serial=ntfs-1A2B3C4D",       // lowercase filesystem
            "serial=NTFS-00000000",       // zero means no serial
            "serial=NTFS-1A2B3C4D+guid=", // clone form without a GUID
            "serial=NTFS-1A2B3C4D+guid={0A1B}",
            "serial=C:-1A2B3C4D",
            "serial=HFS+-1A2B3C4D", // separators in the filesystem name
            "serial=A-B-1A2B3C4D",
            "guid=",
            "guid={}",
            "guid={0A1B-C2D3}",
            "guid=0a1b-c2d3",
            "guid={0a1b}x",
            r"unc=\\NAS\Music",
            r"unc=\\nas",
            r"unc=\\nas\",
            r"unc=\\nas\music\house",
            r"unc=nas\music",
            "dev=",
            "dev=XYZ",
            "uuid=1234",
            "serial=NTFS-1A2B3C4D\n",
        ] {
            assert_eq!(
                VolumeId::from_stored(bad.to_string()),
                Err(BadVolumeId(bad.to_string())),
                "{bad:?} was accepted"
            );
        }
    }

    // The clone rule.

    const GUID_A: &str = "{aaaaaaaa-0000-0000-0000-000000000001}";
    const GUID_B: &str = "{bbbbbbbb-0000-0000-0000-000000000002}";
    const GUID_C: &str = "{cccccccc-0000-0000-0000-000000000003}";

    fn serial_id() -> VolumeId {
        identity(IdentitySignals {
            filesystem: "NTFS",
            ..local(Some(0x1A2B_3C4D), None)
        })
        .unwrap()
    }

    fn seen(id: &VolumeId, mount: &str, label: &str, guid: Option<&str>) -> Sighting {
        Sighting {
            volume: Volume {
                id: id.clone(),
                label: label.into(),
                mount_path: PathBuf::from(mount),
                kind: VolumeKind::External,
            },
            guid: guid.map(str::to_string),
        }
    }

    fn remembered(guid: Option<&str>, label: &str) -> Remembered {
        Remembered {
            id: serial_id(),
            guid: guid.map(str::to_string),
            label: label.into(),
        }
    }

    /// Each mount point's identity after the rule.
    fn ids(scan: &Scan) -> Vec<(String, String)> {
        scan.volumes
            .iter()
            .map(|v| (v.mount_path.display().to_string(), v.id.to_string()))
            .collect()
    }

    fn id_at(scan: &Scan, mount: &str) -> Option<VolumeId> {
        scan.volumes
            .iter()
            .find(|v| v.mount_path == Path::new(mount))
            .map(|v| v.id.clone())
    }

    #[test]
    fn a_volume_whose_serial_no_other_mounted_volume_shares_keeps_its_serial_identity() {
        let scan = tell_clones_apart(
            vec![seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A))],
            &[],
        );
        assert_eq!(id_at(&scan, r"E:\"), Some(serial_id()));
        assert!(scan.clashes.is_empty());
    }

    #[test]
    fn a_clone_plugged_in_alone_answers_to_the_original_drives_identity() {
        // The backup is a drop-in replacement for stored paths.
        let scan = tell_clones_apart(
            vec![seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B))],
            &[remembered(Some(GUID_A), "DJ MUSIC")],
        );
        assert_eq!(id_at(&scan, r"F:\"), Some(serial_id()));
        assert!(scan.clashes.is_empty());
    }

    #[test]
    fn a_backup_the_library_already_told_apart_keeps_its_own_identity_when_plugged_in_alone() {
        let backup = serial_id().with_guid(GUID_B);
        let memory = [
            remembered(Some(GUID_A), "DJ MUSIC"),
            Remembered {
                id: backup.clone(),
                guid: Some(GUID_B.into()),
                label: "DJ MUSIC".into(),
            },
        ];
        let alone = tell_clones_apart(
            vec![seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B))],
            &memory,
        );
        assert_eq!(id_at(&alone, r"F:\"), Some(backup));
        assert!(alone.clashes.is_empty());
        // The original, alone, is still the original.
        let original = tell_clones_apart(
            vec![seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A))],
            &memory,
        );
        assert_eq!(id_at(&original, r"E:\"), Some(serial_id()));
    }

    #[test]
    fn a_backup_the_library_knows_never_keeps_the_shared_identity_even_when_its_label_matches() {
        // The original is away; the known backup B and a relabelled third
        // clone C are plugged in. B's label is the one remembered for the
        // shared identity, but B has an identity of its own.
        let backup = serial_id().with_guid(GUID_B);
        let memory = [
            remembered(Some(GUID_A), "DJ MUSIC"),
            Remembered {
                id: backup.clone(),
                guid: Some(GUID_B.into()),
                label: "DJ MUSIC".into(),
            },
        ];
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B)),
                seen(&serial_id(), r"G:\", "DJ MUSIC 2", Some(GUID_C)),
            ],
            &memory,
        );
        assert_eq!(scan.clashes[0].kept_by, KeptBy::Nobody);
        assert_eq!(id_at(&scan, r"F:\"), Some(backup));
        assert_eq!(id_at(&scan, r"G:\"), Some(serial_id().with_guid(GUID_C)));
    }

    #[test]
    fn a_backup_the_library_knows_never_keeps_the_shared_identity_by_the_remembered_guid() {
        // The shared row's GUID was, wrongly, once the backup's.
        let backup = serial_id().with_guid(GUID_B);
        let memory = [
            remembered(Some(GUID_B), "X"),
            Remembered {
                id: backup.clone(),
                guid: Some(GUID_B.into()),
                label: "X".into(),
            },
        ];
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "A", Some(GUID_A)),
                seen(&serial_id(), r"F:\", "B", Some(GUID_B)),
            ],
            &memory,
        );
        assert_eq!(scan.clashes[0].kept_by, KeptBy::Nobody);
        assert_eq!(id_at(&scan, r"F:\"), Some(backup));
    }

    #[test]
    fn one_volume_at_two_mount_points_is_not_a_clone() {
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
                seen(&serial_id(), r"C:\mnt\usb\", "DJ MUSIC", Some(GUID_A)),
            ],
            &[],
        );
        assert_eq!(id_at(&scan, r"E:\"), Some(serial_id()));
        assert_eq!(id_at(&scan, r"C:\mnt\usb\"), Some(serial_id()));
        assert!(scan.clashes.is_empty());
    }

    #[test]
    fn the_clone_whose_guid_the_library_remembers_keeps_the_serial_identity() {
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
                seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B)),
            ],
            // The remembered label would pick nobody (both match); the GUID decides.
            &[remembered(Some(GUID_B), "DJ MUSIC")],
        );
        assert_eq!(id_at(&scan, r"F:\"), Some(serial_id()));
        assert_eq!(id_at(&scan, r"E:\"), Some(serial_id().with_guid(GUID_A)));
        assert_eq!(scan.clashes[0].kept_by, KeptBy::RememberedGuid);
    }

    #[test]
    fn a_remembered_guid_is_compared_without_regard_to_letter_case() {
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "A", Some(&GUID_A.to_uppercase())),
                seen(&serial_id(), r"F:\", "B", Some(GUID_B)),
            ],
            &[remembered(Some(GUID_A), "")],
        );
        assert_eq!(id_at(&scan, r"E:\"), Some(serial_id()));
        assert_eq!(scan.clashes[0].kept_by, KeptBy::RememberedGuid);
    }

    #[test]
    fn without_a_matching_guid_the_only_clone_with_the_remembered_label_keeps_the_serial_identity()
    {
        // On a new PC the GUIDs are new, but the labels came along.
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
                seen(&serial_id(), r"F:\", "DJ BACKUP", Some(GUID_B)),
            ],
            &[remembered(Some(GUID_C), "DJ MUSIC")],
        );
        assert_eq!(id_at(&scan, r"E:\"), Some(serial_id()));
        assert_eq!(id_at(&scan, r"F:\"), Some(serial_id().with_guid(GUID_B)));
        assert_eq!(scan.clashes[0].kept_by, KeptBy::RememberedLabel);
    }

    #[test]
    fn when_nothing_tells_clones_apart_none_keeps_the_serial_identity_and_the_clash_is_reported() {
        // Same label on both, and the library never saw either GUID (or
        // doesn't know the drive at all).
        for memory in [vec![remembered(Some(GUID_C), "DJ MUSIC")], vec![]] {
            let scan = tell_clones_apart(
                vec![
                    seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
                    seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B)),
                ],
                &memory,
            );
            assert!(scan.volumes.iter().all(|v| v.id != serial_id()), "{scan:?}");
            assert_eq!(
                scan.clashes,
                vec![CloneClash {
                    shared: serial_id(),
                    kept_by: KeptBy::Nobody,
                    left_out: vec![],
                }]
            );
        }
    }

    #[test]
    fn every_clone_that_does_not_keep_the_serial_identity_gets_its_own_valid_identity() {
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
                seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B)),
                seen(&serial_id(), r"G:\", "DJ MUSIC", Some(GUID_C)),
            ],
            &[remembered(Some(GUID_A), "DJ MUSIC")],
        );
        let all: BTreeSet<VolumeId> = scan.volumes.iter().map(|v| v.id.clone()).collect();
        assert_eq!(all.len(), 3, "two clones share an identity: {scan:?}");
        for id in all {
            assert!(!id.as_str().contains(':'), "{id}");
            assert_eq!(VolumeId::from_stored(id.to_string()), Ok(id.clone()));
        }
        assert_eq!(
            id_at(&scan, r"G:\").unwrap().as_str(),
            "serial=NTFS-1A2B3C4D+guid={cccccccc-0000-0000-0000-000000000003}"
        );
    }

    #[test]
    fn a_clone_without_a_guid_is_left_out_not_guessed() {
        let scan = tell_clones_apart(
            vec![
                seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
                seen(&serial_id(), r"F:\", "DJ MUSIC", None),
            ],
            &[remembered(Some(GUID_A), "DJ MUSIC")],
        );
        assert_eq!(ids(&scan), vec![(r"E:\".into(), serial_id().to_string())]);
        assert_eq!(scan.clashes[0].left_out, vec![PathBuf::from(r"F:\")]);
    }

    #[test]
    fn network_shares_and_guid_identities_never_clash() {
        // The same share mapped twice, and a GUID identity at two mount
        // points, are each one volume.
        let nas = network(r"\\nas\music");
        let guid_only = identity(local(None, Some(GUID_A))).unwrap();
        let scan = tell_clones_apart(
            vec![
                seen(&nas, r"\\nas\music\", "", Some(GUID_A)),
                seen(&nas, r"\\nas\music\", "", Some(GUID_B)),
                seen(&guid_only, r"H:\", "", None),
                seen(&guid_only, r"C:\mnt\h\", "", Some(GUID_C)),
            ],
            &[],
        );
        assert!(scan.clashes.is_empty());
        assert!(scan
            .volumes
            .iter()
            .all(|v| v.id == nas || v.id == guid_only));
        assert_eq!(scan.volumes.len(), 4);
    }

    #[test]
    fn the_result_does_not_depend_on_the_order_the_scan_found_volumes() {
        let sightings = vec![
            seen(&serial_id(), r"E:\", "DJ MUSIC", Some(GUID_A)),
            seen(&serial_id(), r"F:\", "DJ MUSIC", Some(GUID_B)),
            seen(&serial_id(), r"G:\", "DJ MUSIC", None),
            seen(&serial_id(), r"C:\mnt\f\", "DJ MUSIC", Some(GUID_B)),
            seen(&network(r"\\nas\music"), r"\\nas\music\", "", None),
            seen(
                &identity(local(Some(7), None)).unwrap(),
                r"K:\",
                "",
                Some(GUID_C),
            ),
        ];
        for memory in [vec![], vec![remembered(Some(GUID_B), "")]] {
            let expected = tell_clones_apart(sightings.clone(), &memory);
            // Every rotation, forwards and backwards.
            for n in 0..sightings.len() {
                let mut order = sightings.clone();
                order.rotate_left(n);
                assert_eq!(tell_clones_apart(order.clone(), &memory), expected);
                order.reverse();
                assert_eq!(tell_clones_apart(order, &memory), expected);
            }
        }
    }
}
