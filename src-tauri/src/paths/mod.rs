//! Stored paths (ROADMAP §0.3): how the app remembers where a file is so
//! that it still finds it after the drive comes back under another letter.
//!
//! A [`StoredPath`] is a volume identity (serial-first, from
//! [`crate::volume`]) plus a [`RelPath`] from that volume's mount point. It
//! never contains a drive letter. To open the file again, [`StoredPath::resolve`]
//! joins the relative path to wherever the volume is mounted *now*.
//!
//! A relative path keeps two spellings:
//! - the exact on-disk name, used to open the file. NTFS doesn't normalize
//!   Unicode, so `Café` spelled NFD and `Café` spelled NFC are two different
//!   files, and only the on-disk spelling opens the right one;
//! - an NFC **match key**, used to compare, dedupe and match against
//!   rekordbox `Location`s, which may spell the same name the other way.
//!
//! Both use `/` as the only separator. Nothing here touches the database;
//! the `volume`, `music_folder` and `file` tables arrive in 0D-1.
//!
//! Names are taken literally. Win32 quietly rewrites plain paths: it drops
//! trailing dots from every component and trailing spaces from the last,
//! so `E:\Q.X.Z.\a.mp3` would open `E:\Q.X.Z\a.mp3`, and it can turn `CON`
//! or `AUX.mp3` into devices. Real artist folders end in dots (Q.X.Z.,
//! N.O.P., Sam Sample Jr.), so every path handed to the OS here is in the
//! `\\?\` form, which Win32 passes through untouched.
//!
//! The rules here are Windows path rules (the only supported platform,
//! ROADMAP §1.1), written as plain string logic so their tests run on every
//! platform. The volume lookup is injected through [`Volumes`], so tests
//! don't need real drives.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::volume::{Sighting, Volume, VolumeId};

#[cfg(windows)]
mod system;
#[cfg(windows)]
pub use self::system::{devices_changed, SystemVolumes};

/// Where volumes are and which volume a path is on. [`SystemVolumes`] asks
/// Windows; tests use a fake.
pub trait Volumes {
    /// The path as the OS really knows it: links, junctions, `subst` and
    /// mapped drives followed, 8.3 short names expanded, on-disk letter
    /// case. The default returns the path unchanged.
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    /// The volume holding `path`, an absolute path in the `\\?\` form.
    fn volume_for(&self, path: &Path) -> io::Result<Volume>;

    /// Where the volume is mounted right now, or `None` if it isn't.
    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf>;

    /// The GUID of the volume mounted as `id` right now, e.g. `{0a1b…}`,
    /// which tells clones apart. The default knows none.
    fn guid(&self, _id: &VolumeId) -> Option<String> {
        None
    }

    /// [`Volumes::volume_for`] plus the volume's GUID, asked of the volume
    /// itself right now. The default knows no GUID.
    fn sighting_for(&self, path: &Path) -> io::Result<Sighting> {
        self.volume_for(path)
            .map(|volume| Sighting { volume, guid: None })
    }
}

/// A file or folder location that survives drive-letter changes: a volume
/// plus a path relative to its mount point.
///
/// Two stored paths are equal when they name the same on-disk spelling;
/// compare [`RelPath::match_key`]s to ignore NFC/NFD differences.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoredPath {
    volume: VolumeId,
    rel: RelPath,
}

impl StoredPath {
    pub fn new(volume: VolumeId, rel: RelPath) -> StoredPath {
        StoredPath { volume, rel }
    }

    /// Turns an absolute path into its stored form.
    ///
    /// Accepts `E:\…`, `\\server\share\…` and their `\\?\` forms, with
    /// either separator. `.` and `..` are resolved first, so a path that
    /// climbs out of a mount point is stored against the volume it really
    /// lands on; a `..` above the drive or share root is rejected. Every
    /// name is taken literally, trailing dots and spaces included.
    pub fn from_absolute(path: &Path, volumes: &impl Volumes) -> Result<StoredPath, PathError> {
        let literal = PathBuf::from(parse_absolute(to_str(path)?)?.verbatim());
        let real = volumes
            .real_path(&literal)
            .map_err(|source| PathError::Unreadable {
                path: literal.clone(),
                source,
            })?;
        let abs = parse_absolute(to_str(&real)?)?;
        let normalized = PathBuf::from(abs.verbatim());
        let volume = volumes
            .volume_for(&normalized)
            .map_err(|source| PathError::Unreadable {
                path: normalized.clone(),
                source,
            })?;
        let mount = parse_absolute(to_str(&volume.mount_path)?)?;
        let parts = abs
            .strip_mount(&mount)
            .ok_or_else(|| PathError::OutsideMount {
                path: normalized.clone(),
                mount: volume.mount_path.clone(),
            })?;
        Ok(StoredPath {
            volume: volume.id,
            rel: RelPath::from_parts(parts)?,
        })
    }

    pub fn volume(&self) -> &VolumeId {
        &self.volume
    }

    pub fn rel(&self) -> &RelPath {
        &self.rel
    }

    /// The absolute path under the volume's current mount point, in the
    /// `\\?\` form so the OS opens exactly this name, or
    /// [`PathError::Offline`] if the volume isn't mounted.
    pub fn resolve(&self, volumes: &impl Volumes) -> Result<PathBuf, PathError> {
        let mount = volumes
            .mount_path(&self.volume)
            .ok_or_else(|| PathError::Offline(self.volume.clone()))?;
        let mut abs = parse_absolute(to_str(&mount)?)?;
        abs.parts.extend(self.rel.components());
        Ok(PathBuf::from(abs.verbatim()))
    }

    /// `rel` inside this folder, e.g. a file inside a music folder.
    pub fn join(&self, rel: &RelPath) -> StoredPath {
        StoredPath {
            volume: self.volume.clone(),
            rel: self.rel.join(rel),
        }
    }

    /// The path from `folder` down to this one, or `None` if this isn't
    /// inside `folder` (or on another volume).
    pub fn relative_to(&self, folder: &StoredPath) -> Option<RelPath> {
        if self.volume != folder.volume {
            return None;
        }
        self.rel.strip_prefix(&folder.rel)
    }
}

/// A path relative to a volume or folder: `/`-separated, never climbing
/// out (`..`), never empty in the middle. The empty path is the root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelPath {
    /// The on-disk spelling.
    raw: String,
    /// `raw` in NFC.
    key: String,
}

impl RelPath {
    /// The root itself: a music folder at the top of a drive.
    pub fn root() -> RelPath {
        RelPath {
            raw: String::new(),
            key: String::new(),
        }
    }

    /// Checks a stored relative path, e.g. one read back from the database.
    /// Rejects `..`, `.`, empty components, `\`, drive prefixes and any
    /// character Windows doesn't allow in a name, so joining it to a mount
    /// point can never land outside that mount point.
    pub fn parse(s: &str) -> Result<RelPath, PathError> {
        if s.is_empty() {
            return Ok(RelPath::root());
        }
        RelPath::from_parts(s.split('/'))
    }

    fn from_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> Result<RelPath, PathError> {
        let mut raw = String::new();
        for part in parts {
            check_component(part)?;
            if !raw.is_empty() {
                raw.push('/');
            }
            raw.push_str(part);
        }
        let key = raw.nfc().collect();
        Ok(RelPath { raw, key })
    }

    /// The on-disk spelling, `/`-separated.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The NFC spelling, for comparing paths whose names may be NFC on one
    /// side and NFD on the other.
    pub fn match_key(&self) -> &str {
        &self.key
    }

    pub fn is_root(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.raw.split('/').filter(|c| !c.is_empty())
    }

    pub fn join(&self, other: &RelPath) -> RelPath {
        let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
            (true, _) => b.to_owned(),
            (_, true) => a.to_owned(),
            _ => format!("{a}/{b}"),
        };
        RelPath {
            raw: join(&self.raw, &other.raw),
            key: join(&self.key, &other.key),
        }
    }

    /// The rest of this path below `base`, compared the way Windows does:
    /// letter case ignored, spelling otherwise exact (an NFC folder name
    /// doesn't match its NFD twin, which is a different folder).
    pub fn strip_prefix(&self, base: &RelPath) -> Option<RelPath> {
        let mut own = self.components();
        for want in base.components() {
            if !same_name(own.next()?, want) {
                return None;
            }
        }
        RelPath::from_parts(own).ok()
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Why a path couldn't be stored or resolved.
#[derive(Debug)]
pub enum PathError {
    /// The path isn't valid Unicode, so it can't be stored or normalized.
    NotUnicode(PathBuf),
    /// Not a full path from a drive or share root (`Music\a.mp3`,
    /// `\Music`, `E:Music`), or a root this app doesn't handle.
    NotAbsolute(String),
    /// A `..` climbs above the drive or share root.
    EscapesRoot(String),
    /// A relative path component that isn't a plain file or folder name.
    BadComponent(String),
    /// The OS couldn't say where the path is (e.g. it doesn't exist).
    Unreadable { path: PathBuf, source: io::Error },
    /// The volume the OS named doesn't contain the path.
    OutsideMount { path: PathBuf, mount: PathBuf },
    /// The volume isn't mounted right now: the drive is unplugged or the
    /// share is unreachable.
    Offline(VolumeId),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::NotUnicode(p) => write!(f, "path {} isn't valid Unicode", p.display()),
            PathError::NotAbsolute(p) => {
                write!(f, "path {p:?} isn't a full path from a drive or share")
            }
            PathError::EscapesRoot(p) => write!(f, "path {p:?} climbs above its root"),
            PathError::BadComponent(c) => write!(f, "{c:?} isn't a valid file or folder name"),
            PathError::Unreadable { path, source } => {
                write!(f, "can't locate {}: {source}", path.display())
            }
            PathError::OutsideMount { path, mount } => write!(
                f,
                "{} isn't under its volume's mount point {}",
                path.display(),
                mount.display()
            ),
            PathError::Offline(id) => write!(f, "volume {id} is offline"),
        }
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PathError::Unreadable { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn to_str(path: &Path) -> Result<&str, PathError> {
    path.to_str()
        .ok_or_else(|| PathError::NotUnicode(path.to_path_buf()))
}

/// `path` as an absolute `\\?\` path with `.` and `..` resolved and every
/// name taken literally: the form the write guard checks and then opens,
/// and the form the sniffer opens files through.
/// Nothing is looked up on disk, so links aren't followed here.
// The write guard and the sniffer use this, and only on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn verbatim_absolute(path: &Path) -> Result<PathBuf, PathError> {
    Ok(PathBuf::from(parse_absolute(to_str(path)?)?.verbatim()))
}

/// Whether `path` is `root` or inside it, compared the way Windows does
/// (drive letter and names ignore letter case, nothing else). Both must be
/// absolute; `.` and `..` are resolved first. Purely textual: resolve links
/// before asking.
// Only the write guard uses this, and only on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_within(path: &Path, root: &Path) -> bool {
    fn parse(p: &Path) -> Option<Absolute<'_>> {
        parse_absolute(p.to_str()?).ok()
    }
    match (parse(path), parse(root)) {
        (Some(path), Some(root)) => path.strip_mount(&root).is_some(),
        _ => false,
    }
}

/// Characters Windows never allows in a file or folder name. `\` and `/`
/// are separators; `:` would also let a component smuggle in a drive
/// (`D:\…`) or an alternate data stream.
fn check_component(part: &str) -> Result<(), PathError> {
    let bad = part.is_empty()
        || part == "."
        || part == ".."
        || part
            .chars()
            .any(|c| c < ' ' || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    if bad {
        Err(PathError::BadComponent(part.to_owned()))
    } else {
        Ok(())
    }
}

/// Whether Windows sees two names as the same: letter case ignored,
/// nothing else. NTFS compares one UTF-16 unit at a time through the
/// volume's upcase table, so there's no Unicode normalization, no
/// multi-letter mapping (`ß` isn't `SS`) and no folding outside the BMP.
/// [`upcase`] follows the same rules; the real table is written per volume
/// at format time and may differ on a few rare letters.
fn same_name(a: &str, b: &str) -> bool {
    a.chars().map(upcase).eq(b.chars().map(upcase))
}

/// A letter's simple uppercase form when the letter and the result are each
/// one UTF-16 unit, as in an NTFS upcase table; otherwise the letter itself.
/// Uppercasing (not lowercasing) keeps the Kelvin sign distinct from `K`.
fn upcase(c: char) -> char {
    if u32::from(c) > 0xFFFF {
        return c;
    }
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) if u32::from(u) <= 0xFFFF => u,
        _ => c,
    }
}

/// The volume whose mount point holds `path` most deeply, as Windows picks
/// it: `C:\mnt\usb\x` is on the volume mounted at `C:\mnt\usb\`, not on C:.
/// The fake volume lookup in the tests uses this rule.
#[cfg(test)]
fn deepest_mount<'v>(path: &Path, volumes: &'v [Volume]) -> Option<&'v Volume> {
    let abs = parse_absolute(path.to_str()?).ok()?;
    volumes
        .iter()
        .filter_map(|v| {
            let mount = parse_absolute(v.mount_path.to_str()?).ok()?;
            abs.strip_mount(&mount)?;
            Some((mount.parts.len(), v))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, v)| v)
}

/// Where an absolute path starts.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Root {
    /// `E:\`, letter uppercased.
    Drive(char),
    /// `\\server\share\`.
    Unc { server: String, share: String },
}

impl Root {
    fn same_as(&self, other: &Root) -> bool {
        match (self, other) {
            (Root::Drive(a), Root::Drive(b)) => a == b,
            (
                Root::Unc { server, share },
                Root::Unc {
                    server: s2,
                    share: sh2,
                },
            ) => same_name(server, s2) && same_name(share, sh2),
            _ => false,
        }
    }
}

/// An absolute Windows path taken apart, with `.` and `..` resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Absolute<'a> {
    root: Root,
    parts: Vec<&'a str>,
}

impl<'a> Absolute<'a> {
    /// The components below `mount`, if this path is on it.
    fn strip_mount(&self, mount: &Absolute<'_>) -> Option<Vec<&'a str>> {
        if !self.root.same_as(&mount.root) || self.parts.len() < mount.parts.len() {
            return None;
        }
        let on_mount = self
            .parts
            .iter()
            .zip(&mount.parts)
            .all(|(a, b)| same_name(a, b));
        on_mount.then(|| self.parts[mount.parts.len()..].to_vec())
    }
}

impl Absolute<'_> {
    /// `\\?\E:\…` or `\\?\UNC\server\share\…`: the form Win32 opens without
    /// rewriting any name.
    fn verbatim(&self) -> String {
        let mut out = match &self.root {
            Root::Drive(letter) => format!(r"\\?\{letter}:\"),
            Root::Unc { server, share } => format!(r"\\?\UNC\{server}\{share}\"),
        };
        out.push_str(&self.parts.join("\\"));
        out
    }
}

/// Parses `E:\…`, `\\server\share\…`, `\\?\E:\…` or `\\?\UNC\server\share\…`,
/// with `\` or `/` separators.
fn parse_absolute(input: &str) -> Result<Absolute<'_>, PathError> {
    let not_absolute = || PathError::NotAbsolute(input.to_owned());
    let is_sep = |c: char| c == '\\' || c == '/';
    // Byte checks only look at ASCII, so slicing after them is safe.
    let sep_byte = |b: u8| b == b'\\' || b == b'/';
    let bytes = input.as_bytes();
    // A `\\?\` (verbatim) or `\\.\` (device namespace) prefix is accepted
    // and dropped; the names after it are taken literally either way. Only
    // `\\?\` also stops Win32 from rewriting names (`\\.\` paths are still
    // normalized), so `\\?\` is the one form this module hands to the OS.
    let verbatim = bytes.len() >= 4
        && sep_byte(bytes[0])
        && sep_byte(bytes[1])
        && matches!(bytes[2], b'?' | b'.')
        && sep_byte(bytes[3]);
    let rest = if verbatim { &input[4..] } else { input };
    let rb = rest.as_bytes();
    let verbatim_unc =
        verbatim && rb.len() >= 4 && rb[..3].eq_ignore_ascii_case(b"UNC") && sep_byte(rb[3]);
    let plain_unc = !verbatim && rb.len() >= 2 && sep_byte(rb[0]) && sep_byte(rb[1]);

    let (root, rest) = if verbatim_unc {
        unc_root(&rest[4..]).ok_or_else(not_absolute)?
    } else if plain_unc {
        unc_root(&rest[2..]).ok_or_else(not_absolute)?
    } else {
        let mut chars = rest.chars();
        match (chars.next(), chars.next(), chars.next()) {
            (Some(letter), Some(':'), Some(sep)) if letter.is_ascii_alphabetic() && is_sep(sep) => {
                (Root::Drive(letter.to_ascii_uppercase()), &rest[3..])
            }
            // `E:` alone is the drive root too.
            (Some(letter), Some(':'), None) if letter.is_ascii_alphabetic() => {
                (Root::Drive(letter.to_ascii_uppercase()), "")
            }
            _ => return Err(not_absolute()),
        }
    };

    let mut parts = Vec::new();
    for part in rest.split(is_sep) {
        match part {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| PathError::EscapesRoot(input.to_owned()))?;
            }
            part => parts.push(part),
        }
    }
    Ok(Absolute { root, parts })
}

/// Splits `server\share\rest` into the root and `rest`.
fn unc_root(s: &str) -> Option<(Root, &str)> {
    let mut it = s.splitn(3, ['\\', '/']);
    let server = it.next().filter(|p| !p.is_empty())?;
    let share = it.next().filter(|p| !p.is_empty())?;
    let root = Root::Unc {
        server: server.to_owned(),
        share: share.to_owned(),
    };
    Some((root, it.next().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::{identity, IdentitySignals, VolumeKind};
    use std::cell::RefCell;

    /// A made-up volume identity, built the same way real ones are.
    fn serial(n: u32) -> VolumeId {
        identity(IdentitySignals {
            kind: VolumeKind::External,
            unc_share: None,
            serial: Some(n),
            filesystem: "NTFS",
            guid: None,
        })
        .unwrap()
    }

    fn share(unc: &str) -> VolumeId {
        identity(IdentitySignals {
            kind: VolumeKind::Network,
            unc_share: Some(unc),
            serial: None,
            filesystem: "",
            guid: None,
        })
        .unwrap()
    }

    /// Volumes and where they're mounted, without real drives. A volume
    /// can be mounted in several places at once; the first is where it
    /// resolves to.
    struct FakeVolumes {
        mounts: Vec<(VolumeId, &'static str)>,
        /// What `volume_for` was asked, to check the paths it receives.
        asked: RefCell<Vec<PathBuf>>,
    }

    impl FakeVolumes {
        fn new(mounts: &[(VolumeId, &'static str)]) -> FakeVolumes {
            FakeVolumes {
                mounts: mounts.to_vec(),
                asked: RefCell::default(),
            }
        }
    }

    impl Volumes for FakeVolumes {
        fn volume_for(&self, path: &Path) -> io::Result<Volume> {
            self.asked.borrow_mut().push(path.to_path_buf());
            let volumes: Vec<Volume> = self
                .mounts
                .iter()
                .map(|(id, m)| Volume {
                    id: id.clone(),
                    label: String::new(),
                    mount_path: PathBuf::from(m),
                    kind: VolumeKind::External,
                })
                .collect();
            deepest_mount(path, &volumes)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such volume"))
        }

        fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
            self.mounts
                .iter()
                .find(|(v, _)| v == id)
                .map(|(_, m)| PathBuf::from(m))
        }
    }

    fn usb() -> VolumeId {
        serial(0x1A2B_3C4D)
    }

    fn system() -> VolumeId {
        serial(0x0000_C0DE)
    }

    fn store(path: &str, volumes: &FakeVolumes) -> StoredPath {
        StoredPath::from_absolute(Path::new(path), volumes)
            .unwrap_or_else(|e| panic!("storing {path:?}: {e}"))
    }

    fn resolved(stored: &StoredPath, volumes: &FakeVolumes) -> String {
        let path = stored.resolve(volumes).unwrap();
        path.to_str().unwrap().to_owned()
    }

    /// The `\\?\` form of a plain absolute path, as `resolve` returns it.
    fn verbatim(plain: &str) -> String {
        match plain.strip_prefix(r"\\") {
            Some(unc) => format!(r"\\?\UNC\{unc}"),
            None => format!(r"\\?\{plain}"),
        }
    }

    #[test]
    fn a_stored_path_resolves_back_to_the_same_absolute_path() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let path = r"E:\Music\House\Artist - Title (Extended Mix).mp3";
        let stored = store(path, &volumes);
        assert_eq!(stored.volume(), &usb());
        assert_eq!(
            stored.rel().as_str(),
            "Music/House/Artist - Title (Extended Mix).mp3"
        );
        assert_eq!(resolved(&stored, &volumes), verbatim(path));
    }

    #[test]
    fn awkward_names_from_real_rekordbox_libraries_round_trip_unchanged() {
        // ROADMAP §5.2/§5.3: accents, CJK, emoji, `# % + & '`, and the raw
        // `( ) ,` that rekordbox leaves in Locations.
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let names = [
            r"E:\Música\Artisté\Déjà Nu (Remix).flac",
            r"E:\Music\坂本龍一\戦場のメリークリスマス.aiff",
            r"E:\Music\ＦＵＬＬ ＷＩＤＴＨ\안녕하세요.wav",
            r"E:\Music\🔥 Bangers 🔥\💿 Track #1.mp3",
            r"E:\Music\#hashtag\100% Pure + Live & Loud's.mp3",
            r"E:\Music\Artist, The\Title (feat. X) [VIP].m4a",
            r"E:\Music\a%20b\c%2Fd.mp3",
        ];
        for path in names {
            let stored = store(path, &volumes);
            assert_eq!(resolved(&stored, &volumes), verbatim(path));
            // What's stored parses back to the same thing, as when it's
            // read from the database.
            assert_eq!(
                RelPath::parse(stored.rel().as_str()).unwrap(),
                *stored.rel()
            );
        }
    }

    #[test]
    fn stored_relative_paths_use_forward_slashes_whatever_the_input_used() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let a = store(r"E:\Music\House\a.mp3", &volumes);
        let b = store("E:/Music/House/a.mp3", &volumes);
        let c = store(r"E:\Music/House\\a.mp3", &volumes);
        assert_eq!(a.rel().as_str(), "Music/House/a.mp3");
        assert_eq!(a, b);
        assert_eq!(a, c);
        // Resolving always gives backslashes.
        assert_eq!(resolved(&b, &volumes), verbatim(r"E:\Music\House\a.mp3"));
    }

    #[test]
    fn nfd_input_gets_an_nfc_match_key_and_keeps_its_on_disk_spelling() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let nfd = "E:\\Music\\Cafe\u{301} del Mar.mp3";
        let stored = store(nfd, &volumes);
        assert_eq!(stored.rel().match_key(), "Music/Caf\u{e9} del Mar.mp3");
        // The on-disk spelling is kept so the file still opens: NTFS
        // doesn't normalize names.
        assert_eq!(stored.rel().as_str(), "Music/Cafe\u{301} del Mar.mp3");
        assert_eq!(resolved(&stored, &volumes), verbatim(nfd));
    }

    #[test]
    fn nfd_and_nfc_spellings_in_one_folder_stay_two_files_with_one_match_key() {
        // NTFS keeps both as separate files, so each must resolve to its own.
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let nfd_path = "E:\\Music\\Cafe\u{301}.mp3";
        let nfc_path = "E:\\Music\\Caf\u{e9}.mp3";
        let nfd = store(nfd_path, &volumes);
        let nfc = store(nfc_path, &volumes);
        assert_ne!(nfd, nfc);
        assert_eq!(resolved(&nfd, &volumes), verbatim(nfd_path));
        assert_eq!(resolved(&nfc, &volumes), verbatim(nfc_path));
        assert_eq!(nfd.rel().match_key(), nfc.rel().match_key());
        assert_eq!(nfc.rel().match_key(), "Music/Caf\u{e9}.mp3");
    }

    #[test]
    fn the_same_file_under_a_different_drive_letter_has_the_same_stored_form() {
        let at_e = FakeVolumes::new(&[(usb(), r"E:\")]);
        let at_f = FakeVolumes::new(&[(usb(), r"F:\")]);
        assert_eq!(
            store(r"E:\Music\a.mp3", &at_e),
            store(r"F:\Music\a.mp3", &at_f)
        );
    }

    #[test]
    fn a_drive_whose_letter_changed_resolves_under_its_new_letter() {
        let at_e = FakeVolumes::new(&[(usb(), r"E:\")]);
        let stored = store(r"E:\Music\a.mp3", &at_e);
        let at_f = FakeVolumes::new(&[(usb(), r"F:\")]);
        assert_eq!(resolved(&stored, &at_f), verbatim(r"F:\Music\a.mp3"));
    }

    #[test]
    fn a_verbatim_prefix_gives_the_same_stored_form() {
        let volumes =
            FakeVolumes::new(&[(usb(), r"E:\"), (share(r"\\nas\music"), r"\\nas\music\")]);
        assert_eq!(
            store(r"\\?\E:\Music\a.mp3", &volumes),
            store(r"E:\Music\a.mp3", &volumes)
        );
        assert_eq!(
            store(r"\\?\UNC\nas\music\House\a.mp3", &volumes),
            store(r"\\nas\music\House\a.mp3", &volumes)
        );
        assert_eq!(
            store(r"\\.\E:\Music\a.mp3", &volumes),
            store(r"E:\Music\a.mp3", &volumes)
        );
        // The volume lookup always gets the one `\\?\` form.
        for asked in volumes.asked.borrow().iter() {
            let asked = asked.to_str().unwrap();
            assert!(asked.starts_with(r"\\?\"), "{asked:?}");
            assert!(!asked.contains('/'), "{asked:?}");
        }
    }

    #[test]
    fn names_with_trailing_dots_or_spaces_and_device_names_are_kept_literally() {
        // Win32 would open `Q.X.Z` for `Q.X.Z.`, `b.mp3` for `b.mp3.`, and
        // a device for `CON`. The `\\?\` form stops all of that.
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        for (path, rel) in [
            (r"E:\Q.X.Z.\Album One\a.mp3", "Q.X.Z./Album One/a.mp3"),
            (r"E:\Sam Sample Jr.\a.mp3", "Sam Sample Jr./a.mp3"),
            (r"E:\Music\b.mp3.", "Music/b.mp3."),
            (r"E:\Music\c.mp3 ", "Music/c.mp3 "),
            (r"E:\Music\CON", "Music/CON"),
            (r"E:\Music\AUX.mp3", "Music/AUX.mp3"),
        ] {
            let stored = store(path, &volumes);
            assert_eq!(stored.rel().as_str(), rel);
            assert_eq!(resolved(&stored, &volumes), verbatim(path));
            let asked = volumes.asked.borrow().last().unwrap().clone();
            assert_eq!(asked, Path::new(&verbatim(path)));
        }
        assert_ne!(
            store(r"E:\Q.X.Z.\a.mp3", &volumes),
            store(r"E:\Q.X.Z\a.mp3", &volumes)
        );
    }

    #[test]
    fn names_compare_like_ntfs_letter_case_only() {
        assert!(same_name("DJ Music", "dj music"));
        assert!(same_name("Déjà Nu", "DÉJÀ NU"));
        assert!(same_name("\u{3c3}\u{3c2}", "\u{3a3}\u{3a3}")); // σς / ΣΣ
                                                                // Different names to NTFS:
        assert!(!same_name("Caf\u{e9}", "Cafe\u{301}")); // NFC vs NFD
        assert!(!same_name("K", "\u{212a}")); // Kelvin sign
        assert!(!same_name("stra\u{df}e", "STRASSE")); // ß isn't SS
        assert!(!same_name("\u{10428}", "\u{10400}")); // outside the BMP
    }

    #[test]
    fn a_folder_does_not_claim_files_under_its_nfd_twin() {
        // `Café` (NFC) and `Café` (NFD) are two folders on NTFS; joining a
        // file of one to the other would point at a different file.
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let nfc_folder = store("E:\\Caf\u{e9}", &volumes);
        let nfd_file = store("E:\\Cafe\u{301}\\a.mp3", &volumes);
        assert_eq!(nfd_file.relative_to(&nfc_folder), None);
        let nfd_folder = store("E:\\Cafe\u{301}", &volumes);
        assert_eq!(nfd_file.relative_to(&nfd_folder).unwrap().as_str(), "a.mp3");
    }

    #[test]
    fn the_deepest_mount_point_holding_a_path_wins() {
        let volume = |id: VolumeId, mount: &str| Volume {
            id,
            label: String::new(),
            mount_path: PathBuf::from(mount),
            kind: VolumeKind::External,
        };
        let mounted = [volume(system(), r"C:\"), volume(usb(), r"C:\mnt\usb\")];
        let on = |p: &str| deepest_mount(Path::new(p), &mounted).map(|v| v.id.clone());
        assert_eq!(on(r"\\?\C:\mnt\usb\a.mp3"), Some(usb()));
        assert_eq!(on(r"\\?\C:\MNT\USB"), Some(usb()));
        assert_eq!(on(r"\\?\C:\mnt\usb2\a.mp3"), Some(system()));
        assert_eq!(on(r"\\?\D:\a.mp3"), None);
    }

    #[test]
    fn drive_letter_and_mount_folder_case_do_not_matter() {
        let volumes = FakeVolumes::new(&[(system(), r"C:\"), (usb(), r"C:\Mnt\USB\")]);
        assert_eq!(
            store(r"e:\Music\a.mp3", &FakeVolumes::new(&[(usb(), r"E:\")])),
            store(r"c:\mnt\usb\Music\a.mp3", &volumes)
        );
    }

    #[test]
    fn a_volume_mounted_in_a_folder_stores_the_same_as_under_a_drive_letter() {
        let volumes =
            FakeVolumes::new(&[(usb(), r"E:\"), (system(), r"C:\"), (usb(), r"C:\mnt\usb\")]);
        let by_letter = store(r"E:\Music\a.mp3", &volumes);
        let by_folder = store(r"C:\mnt\usb\Music\a.mp3", &volumes);
        assert_eq!(by_letter, by_folder);
        assert_eq!(by_folder.volume(), &usb());
    }

    #[test]
    fn a_network_share_round_trips() {
        let nas = share(r"\\NAS\Music");
        let volumes = FakeVolumes::new(&[(nas.clone(), r"\\nas\music\")]);
        let stored = store(r"\\nas\music\House\a.mp3", &volumes);
        assert_eq!(stored.volume(), &nas);
        assert_eq!(stored.rel().as_str(), "House/a.mp3");
        assert_eq!(
            resolved(&stored, &volumes),
            verbatim(r"\\nas\music\House\a.mp3")
        );
    }

    #[test]
    fn a_file_at_the_volume_root_and_the_root_itself_round_trip() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let file = store(r"E:\a.mp3", &volumes);
        assert_eq!(file.rel().as_str(), "a.mp3");
        assert_eq!(resolved(&file, &volumes), verbatim(r"E:\a.mp3"));

        for root in [r"E:\", "E:", r"\\?\E:\"] {
            let stored = store(root, &volumes);
            assert!(stored.rel().is_root(), "{root}");
            assert_eq!(resolved(&stored, &volumes), verbatim(r"E:\"));
        }
    }

    #[test]
    fn a_volume_that_is_not_mounted_resolves_to_an_offline_error() {
        let stored = store(r"E:\Music\a.mp3", &FakeVolumes::new(&[(usb(), r"E:\")]));
        let unplugged = FakeVolumes::new(&[(system(), r"C:\")]);
        match stored.resolve(&unplugged) {
            Err(PathError::Offline(id)) => assert_eq!(id, usb()),
            other => panic!("expected Offline, got {other:?}"),
        }
        let message = stored.resolve(&unplugged).unwrap_err().to_string();
        assert!(message.contains("offline"), "{message}");
    }

    #[test]
    fn dot_segments_are_resolved_before_the_volume_is_chosen() {
        // `C:\mnt\usb\..\secret` is on C:, not on the USB drive mounted
        // at C:\mnt\usb.
        let volumes = FakeVolumes::new(&[(system(), r"C:\"), (usb(), r"C:\mnt\usb\")]);
        let stored = store(r"C:\mnt\usb\..\secret\.\a.mp3", &volumes);
        assert_eq!(stored.volume(), &system());
        assert_eq!(stored.rel().as_str(), "mnt/secret/a.mp3");
        assert_eq!(
            volumes.asked.borrow().last().unwrap(),
            Path::new(r"\\?\C:\mnt\secret\a.mp3")
        );
    }

    #[test]
    fn a_path_climbing_above_its_drive_or_share_root_is_rejected() {
        let volumes =
            FakeVolumes::new(&[(usb(), r"E:\"), (share(r"\\nas\music"), r"\\nas\music\")]);
        for path in [
            r"E:\..\a.mp3",
            r"E:\Music\..\..\a.mp3",
            r"\\nas\music\..\other\a.mp3",
            r"\\?\E:\..\a.mp3",
        ] {
            match StoredPath::from_absolute(Path::new(path), &volumes) {
                Err(PathError::EscapesRoot(_)) => {}
                other => panic!("{path}: expected EscapesRoot, got {other:?}"),
            }
        }
    }

    #[test]
    fn relative_and_drive_relative_paths_are_rejected() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        for path in [
            r"Music\a.mp3",
            r"\Music\a.mp3",
            r"E:Music\a.mp3",
            "",
            r"\\server",
            r"\\server\",
            r"\\?\Volume{0a1b}\a.mp3",
        ] {
            match StoredPath::from_absolute(Path::new(path), &volumes) {
                Err(PathError::NotAbsolute(_)) => {}
                other => panic!("{path:?}: expected NotAbsolute, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_stored_relative_path_that_could_escape_its_root_is_rejected() {
        // What's read back from the database is checked again before it's
        // joined to a mount point.
        for bad in [
            "..",
            "../Windows/System32",
            "Music/../../Windows",
            "Music/./a.mp3",
            "/Music/a.mp3",
            "Music/",
            "Music//a.mp3",
            r"Music\..\..\a.mp3",
            r"D:\Windows",
            "D:",
            "a.mp3:hidden_stream",
            "a?.mp3",
            "a\u{0}.mp3",
        ] {
            match RelPath::parse(bad) {
                Err(PathError::BadComponent(_)) => {}
                other => panic!("{bad:?}: expected BadComponent, got {other:?}"),
            }
        }
        assert!(RelPath::parse("").unwrap().is_root());
        assert_eq!(
            RelPath::parse("Music/..a/b..").unwrap().as_str(),
            "Music/..a/b.."
        );
    }

    #[test]
    fn a_failed_volume_lookup_is_an_error_not_a_panic() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        match StoredPath::from_absolute(Path::new(r"Q:\Music\a.mp3"), &volumes) {
            Err(PathError::Unreadable { path, .. }) => {
                assert_eq!(path, Path::new(r"\\?\Q:\Music\a.mp3"))
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn a_volume_whose_mount_point_does_not_contain_the_path_is_an_error() {
        struct Liar;
        impl Volumes for Liar {
            fn volume_for(&self, _: &Path) -> io::Result<Volume> {
                Ok(Volume {
                    id: serial(1),
                    label: String::new(),
                    mount_path: PathBuf::from(r"F:\"),
                    kind: VolumeKind::External,
                })
            }
            fn mount_path(&self, _: &VolumeId) -> Option<PathBuf> {
                None
            }
        }
        match StoredPath::from_absolute(Path::new(r"E:\a.mp3"), &Liar) {
            Err(PathError::OutsideMount { .. }) => {}
            other => panic!("expected OutsideMount, got {other:?}"),
        }
    }

    #[test]
    fn the_real_path_is_used_so_links_resolve_to_where_the_file_is() {
        // A junction C:\Links\Music → E:\Music: the stored form is the
        // target's.
        struct Junction(FakeVolumes);
        impl Volumes for Junction {
            fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
                let s = path.to_str().unwrap();
                Ok(PathBuf::from(s.replace(r"C:\Links\Music", r"E:\Music")))
            }
            fn volume_for(&self, path: &Path) -> io::Result<Volume> {
                self.0.volume_for(path)
            }
            fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
                self.0.mount_path(id)
            }
        }
        let volumes = Junction(FakeVolumes::new(&[(system(), r"C:\"), (usb(), r"E:\")]));
        let via_link =
            StoredPath::from_absolute(Path::new(r"C:\Links\Music\a.mp3"), &volumes).unwrap();
        assert_eq!(via_link.volume(), &usb());
        assert_eq!(via_link.rel().as_str(), "Music/a.mp3");
    }

    #[test]
    fn a_file_is_stored_relative_to_its_music_folder_and_joins_back() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\")]);
        let folder = store(r"E:\DJ Music", &volumes);
        let file = store(r"E:\DJ Music\House\Déjà Nu.mp3", &volumes);
        let rel = file.relative_to(&folder).unwrap();
        assert_eq!(rel.as_str(), "House/Déjà Nu.mp3");
        assert_eq!(folder.join(&rel), file);
        // Letter case in the folder part doesn't matter, as on Windows.
        let upper = store(r"E:\DJ MUSIC", &volumes);
        assert_eq!(file.relative_to(&upper).unwrap(), rel);
        // A folder at the root of the drive.
        let root = store(r"E:\", &volumes);
        assert_eq!(file.relative_to(&root).unwrap(), *file.rel());
        assert_eq!(root.join(file.rel()), file);
    }

    #[test]
    fn a_file_outside_a_folder_or_on_another_volume_is_not_relative_to_it() {
        let volumes = FakeVolumes::new(&[(usb(), r"E:\"), (system(), r"C:\")]);
        let folder = store(r"E:\DJ Music", &volumes);
        // A sibling whose name merely starts the same.
        assert_eq!(
            store(r"E:\DJ Music Old\a.mp3", &volumes).relative_to(&folder),
            None
        );
        assert_eq!(
            store(r"E:\Other\a.mp3", &volumes).relative_to(&folder),
            None
        );
        assert_eq!(
            store(r"C:\DJ Music\a.mp3", &volumes).relative_to(&folder),
            None
        );
    }

    #[test]
    fn joined_paths_keep_both_spellings_in_step() {
        let folder = RelPath::parse("Cafe\u{301}").unwrap();
        let file = RelPath::parse("a.mp3").unwrap();
        let joined = folder.join(&file);
        assert_eq!(joined.as_str(), "Cafe\u{301}/a.mp3");
        assert_eq!(joined.match_key(), "Caf\u{e9}/a.mp3");
        assert_eq!(RelPath::root().join(&file), file);
        assert_eq!(file.join(&RelPath::root()), file);
    }
}
