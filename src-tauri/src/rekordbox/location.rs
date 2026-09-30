//! rekordbox `Location` values, decoded by hand (ROADMAP §5.3).
//!
//! rekordbox writes a track's file as `file://localhost/C:/Music/a%20b.mp3`,
//! but not as a standard file address: it leaves `# ( ) , + !` raw and
//! writes lowercase `%xx`. A general-purpose address parser cuts the path at
//! the `#` and may turn `+` into a space, so this module decodes it itself:
//!
//! - Only `%xx` is special (either letter case). Every other character,
//!   `#` and `+` included, stands for itself.
//! - The decoded bytes must be UTF-8; multi-byte characters arrive as runs
//!   of `%xx` (`%c3%a9` is `é`), or raw when the writer left them unencoded.
//! - The path is then classified: a Windows drive path (`C:/…`), a Windows
//!   network path (`//server/share/…`), or a macOS path (`/Users/…`).
//! - Entries that aren't files at all (`soundcloud:tracks:1`, `spotify:…`)
//!   come back as [`Location::Streaming`], never as an error: they're kept
//!   as "streaming, no file" (ROADMAP 1.2).
//!
//! Bad input is an error value ([`LocationError`]), never a panic.
//! Matching a decoded path against files uses [`FilePath::match_key`].

use std::fmt;

use unicode_normalization::UnicodeNormalization;

/// What a `Location` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A file on disk.
    File(FilePath),
    /// A streaming service's track, which has no file.
    Streaming(StreamingId),
}

impl Location {
    /// A key for matching one `Location` against another spelling of it
    /// (a playlist entry against a COLLECTION track): see
    /// [`FilePath::match_key`]. A streaming entry's key is its scheme,
    /// lowercased, and its id as written.
    pub fn match_key(&self) -> String {
        match self {
            Location::File(path) => path.match_key(),
            Location::Streaming(id) => format!("{}:{}", id.scheme, id.id),
        }
    }

    pub fn as_file(&self) -> Option<&FilePath> {
        match self {
            Location::File(path) => Some(path),
            Location::Streaming(_) => None,
        }
    }
}

/// A streaming entry: `soundcloud:tracks:12345` is scheme `soundcloud`,
/// id `tracks:12345`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingId {
    /// The service's scheme, lowercased: `soundcloud`, `spotify`, `tidal`…
    pub scheme: String,
    /// Everything after the scheme's colon, as written.
    pub id: String,
}

/// Which kind of path a file `Location` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// `C:/Music/a.mp3`.
    WindowsDrive,
    /// `//server/share/Music/a.mp3`, a Windows network (UNC) path.
    WindowsUnc,
    /// `/Users/someone/Music/a.mp3`, from rekordbox on a Mac.
    Posix,
}

/// A decoded file path, `/`-separated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePath {
    style: PathStyle,
    path: String,
}

impl FilePath {
    pub fn style(&self) -> PathStyle {
        self.style
    }

    /// The path with `/` separators, spelled exactly as decoded:
    /// `C:/Music/a.mp3`, `//server/share/a.mp3`, `/Users/a.mp3`.
    pub fn as_str(&self) -> &str {
        &self.path
    }

    /// The path as Windows writes it (`C:\Music\a.mp3`,
    /// `\\server\share\a.mp3`), or `None` for a macOS path.
    pub fn to_windows(&self) -> Option<String> {
        match self.style {
            PathStyle::WindowsDrive | PathStyle::WindowsUnc => Some(self.path.replace('/', "\\")),
            PathStyle::Posix => None,
        }
    }

    /// The last component: the file's name.
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }

    /// The path's components below its root: for `C:/Music/a.mp3`,
    /// `Music` and `a.mp3`; for `//server/share/a.mp3`, `a.mp3`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        let skip = match self.style {
            PathStyle::WindowsDrive => 1,
            PathStyle::WindowsUnc => 2,
            PathStyle::Posix => 0,
        };
        let rest = self.path.trim_start_matches('/');
        rest.split('/').skip(skip).filter(|c| !c.is_empty())
    }

    /// A key for comparing two spellings of the same path: NFC, and for
    /// Windows paths, letter case ignored the way NTFS ignores it (each
    /// letter's simple uppercase form within the BMP; `ß` stays `ß`).
    /// macOS paths keep their case.
    pub fn match_key(&self) -> String {
        let nfc = self.path.nfc();
        match self.style {
            PathStyle::WindowsDrive | PathStyle::WindowsUnc => nfc.map(upcase).collect(),
            PathStyle::Posix => nfc.collect(),
        }
    }
}

impl fmt::Display for FilePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)
    }
}

/// Why a `Location` couldn't be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationError {
    /// The value is empty (or the attribute is missing).
    Empty,
    /// No `scheme:` at the start, so it's neither a file nor a streaming
    /// entry.
    NoScheme,
    /// A bare Windows path (`C:/Music/a.mp3`) where a `file://` address
    /// belongs.
    BarePath,
    /// `file:` not followed by `//`.
    BadFilePrefix,
    /// A `%` not followed by two hex digits, at this byte of the value.
    BadEscape { at: usize },
    /// The decoded bytes aren't UTF-8.
    NotUtf8,
    /// The decoded path holds a NUL character, which no file name can.
    NulByte,
    /// `file://localhost/` with nothing after it.
    NoPath,
    /// A network path without a server or share name.
    BadNetworkPath,
}

impl fmt::Display for LocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocationError::Empty => f.write_str("empty Location"),
            LocationError::NoScheme => f.write_str("no scheme"),
            LocationError::BarePath => f.write_str("bare path, not a file:// address"),
            LocationError::BadFilePrefix => f.write_str("file: not followed by //"),
            LocationError::BadEscape { at } => write!(f, "bad %-escape at byte {at}"),
            LocationError::NotUtf8 => f.write_str("decodes to bytes that aren't UTF-8"),
            LocationError::NulByte => f.write_str("decodes to a NUL character"),
            LocationError::NoPath => f.write_str("no path after the file:// prefix"),
            LocationError::BadNetworkPath => f.write_str("network path without server or share"),
        }
    }
}

impl std::error::Error for LocationError {}

/// Decodes a `Location` value (already XML-unescaped: `&amp;` is `&`).
pub fn decode(raw: &str) -> Result<Location, LocationError> {
    if raw.is_empty() {
        return Err(LocationError::Empty);
    }
    let colon = raw.find(':').ok_or(LocationError::NoScheme)?;
    let scheme = &raw[..colon];
    if !is_scheme(scheme) {
        return Err(LocationError::NoScheme);
    }
    if !scheme.eq_ignore_ascii_case("file") {
        // One letter before the colon is a drive letter, not a scheme.
        if scheme.len() == 1 {
            return Err(LocationError::BarePath);
        }
        return Ok(Location::Streaming(StreamingId {
            scheme: scheme.to_ascii_lowercase(),
            id: raw[colon + 1..].to_owned(),
        }));
    }

    let after = raw[colon + 1..]
        .strip_prefix("//")
        .ok_or(LocationError::BadFilePrefix)?;
    let offset = colon + 3;
    // The host runs to the next `/`: `localhost` or empty for this
    // computer, anything else names a server.
    let host_end = after.find('/').unwrap_or(after.len());
    let host = percent_decode(&after[..host_end], offset)?;
    let path = percent_decode(&after[host_end..], offset + host_end)?;
    if is_drive(&host) {
        // `file://C:/Music/a.mp3`: the drive where the host belongs.
        return classify(&format!("/{host}{path}"));
    }
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        // `file://server/share/a.mp3`.
        return network_path(&format!("{host}{path}"));
    }
    classify(&path)
}

/// Sorts a decoded local path (it starts with `/`, or is empty) into its
/// kind.
fn classify(path: &str) -> Result<Location, LocationError> {
    let Some(rest) = path.strip_prefix('/') else {
        return Err(LocationError::NoPath);
    };
    if rest.is_empty() {
        return Err(LocationError::NoPath);
    }
    // `file://localhost//server/share/…` or `file://localhost/\\server\…`.
    // (More slashes, `file://///…`, read as a network path without a
    // server, which is an error; rekordbox doesn't write that.)
    if let Some(unc) = rest.strip_prefix('/').or_else(|| rest.strip_prefix("\\\\")) {
        return network_path(&unc.replace('\\', "/"));
    }
    if is_drive(rest) {
        return Ok(Location::File(FilePath {
            style: PathStyle::WindowsDrive,
            path: rest.replace('\\', "/"),
        }));
    }
    // A `scheme:` of two or more letters can't start a Windows path (no `:`
    // in names), so this is a streaming entry written with the file prefix:
    // `file://localhost/soundcloud:tracks:1`. A macOS file at the top of
    // the disk with `:` in its name (`/Artist: Title.mp3`) would read as
    // streaming too; rekordbox doesn't write one there.
    if let Some(colon) = rest.find(':') {
        let scheme = &rest[..colon];
        if scheme.len() > 1 && is_scheme(scheme) {
            return Ok(Location::Streaming(StreamingId {
                scheme: scheme.to_ascii_lowercase(),
                id: rest[colon + 1..].to_owned(),
            }));
        }
    }
    Ok(Location::File(FilePath {
        style: PathStyle::Posix,
        path: path.to_owned(),
    }))
}

/// `server/share[/…]`, the part after the leading `//`, with `/`
/// separators.
fn network_path(path: &str) -> Result<Location, LocationError> {
    let mut parts = path.splitn(3, '/');
    let server = parts.next().unwrap_or("");
    let share = parts.next().unwrap_or("");
    if server.is_empty() || share.is_empty() {
        return Err(LocationError::BadNetworkPath);
    }
    let rest = parts.next();
    let path = match rest {
        Some(rest) => format!("//{server}/{share}/{rest}"),
        None => format!("//{server}/{share}"),
    };
    Ok(Location::File(FilePath {
        style: PathStyle::WindowsUnc,
        path,
    }))
}

/// `C:` alone, or followed by a separator.
fn is_drive(rest: &str) -> bool {
    let b = rest.as_bytes();
    b.len() >= 2
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b.len() == 2 || b[2] == b'/' || b[2] == b'\\')
}

/// A scheme: a letter, then letters, digits, `+`, `-` or `.`.
fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Replaces each `%xx` (either case) with its byte; everything else stays
/// as it is. `offset` is where `s` starts in the whole value, for errors.
fn percent_decode(s: &str, offset: usize) -> Result<String, LocationError> {
    let bytes = s.as_bytes();
    if !bytes.contains(&b'%') {
        if s.contains('\0') {
            return Err(LocationError::NulByte);
        }
        return Ok(s.to_owned());
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1).copied().and_then(hex_value);
            let lo = bytes.get(i + 2).copied().and_then(hex_value);
            match (hi, lo) {
                (Some(hi), Some(lo)) => out.push(hi << 4 | lo),
                _ => return Err(LocationError::BadEscape { at: offset + i }),
            }
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).map_err(|_| LocationError::NotUtf8)?;
    if decoded.contains('\0') {
        return Err(LocationError::NulByte);
    }
    Ok(decoded)
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A letter's simple uppercase form when the letter and the result are
/// each one UTF-16 unit, as in an NTFS upcase table; otherwise the letter
/// itself. (The same rule as `paths`' private `upcase`.)
fn upcase(c: char) -> char {
    if u32::from(c) > 0xFFFF {
        return c;
    }
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(u), None) if u32::from(u) <= 0xFFFF => u,
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn file(raw: &str) -> FilePath {
        match decode(raw) {
            Ok(Location::File(path)) => path,
            other => panic!("{raw:?} decoded to {other:?}"),
        }
    }

    fn drive(raw: &str) -> String {
        let path = file(raw);
        assert_eq!(path.style(), PathStyle::WindowsDrive, "{raw:?}");
        path.as_str().to_owned()
    }

    #[test]
    fn a_plain_windows_location_decodes_to_its_drive_path() {
        assert_eq!(
            drive("file://localhost/C:/Users/someone/Music/a%20b.mp3"),
            "C:/Users/someone/Music/a b.mp3"
        );
        assert_eq!(
            file("file://localhost/C:/Music/a%20b.mp3").to_windows(),
            Some("C:\\Music\\a b.mp3".to_owned())
        );
    }

    #[test]
    fn a_raw_hash_is_part_of_the_name_not_a_fragment() {
        assert_eq!(
            drive("file://localhost/C:/Kit/Low%20Tide%20#1.wav"),
            "C:/Kit/Low Tide #1.wav"
        );
        assert_eq!(drive("file://localhost/C:/#/##.mp3"), "C:/#/##.mp3");
    }

    #[test]
    fn a_raw_plus_is_a_literal_plus_not_a_space() {
        assert_eq!(drive("file://localhost/C:/A+B.mp3"), "C:/A+B.mp3");
        assert_eq!(drive("file://localhost/C:/A%2BB.mp3"), "C:/A+B.mp3");
        assert_eq!(drive("file://localhost/C:/A%20+%20B.mp3"), "C:/A + B.mp3");
    }

    #[test]
    fn raw_parentheses_commas_and_bangs_stay_as_written() {
        assert_eq!(
            drive("file://localhost/C:/North%20Wind,%20Late%20(Flip)!.m4a"),
            "C:/North Wind, Late (Flip)!.m4a"
        );
    }

    #[test]
    fn upper_and_lowercase_hex_escapes_decode_alike() {
        assert_eq!(
            drive("file://localhost/C:/%5bx%5d%2a.mp3"),
            drive("file://localhost/C:/%5Bx%5D%2A.mp3")
        );
        assert_eq!(drive("file://localhost/C:/%5bx%5D.mp3"), "C:/[x].mp3");
    }

    #[test]
    fn an_escaped_percent_decodes_once_not_twice() {
        assert_eq!(
            drive("file://localhost/C:/100%25%20Flip.wav"),
            "C:/100% Flip.wav"
        );
        assert_eq!(drive("file://localhost/C:/%2541.mp3"), "C:/%41.mp3");
    }

    #[test]
    fn multi_byte_utf8_sequences_decode_to_their_characters() {
        // é (2 bytes), 日本 (3 bytes each), 🚀 (4 bytes).
        assert_eq!(
            drive("file://localhost/C:/Caf%c3%a9%20%e6%97%a5%e6%9c%ac%20%f0%9f%9a%80.m4a"),
            "C:/Café 日本 🚀.m4a"
        );
    }

    #[test]
    fn raw_non_ascii_characters_are_accepted_as_written() {
        assert_eq!(
            drive("file://localhost/C:/Café 日本.mp3"),
            "C:/Café 日本.mp3"
        );
    }

    #[test]
    fn escapes_that_split_a_character_or_arent_utf8_are_errors() {
        assert_eq!(
            decode("file://localhost/C:/%c3.mp3"),
            Err(LocationError::NotUtf8)
        );
        assert_eq!(
            decode("file://localhost/C:/%ff%fe.mp3"),
            Err(LocationError::NotUtf8)
        );
        assert_eq!(
            decode("file://localhost/C:/%ed%a0%80.mp3"),
            Err(LocationError::NotUtf8),
            "an encoded UTF-16 surrogate isn't UTF-8"
        );
    }

    #[test]
    fn malformed_escapes_are_errors_with_their_position() {
        let prefix = "file://localhost/C:/".len();
        assert_eq!(
            decode("file://localhost/C:/a%zz.mp3"),
            Err(LocationError::BadEscape { at: prefix + 1 })
        );
        assert_eq!(
            decode("file://localhost/C:/a%4"),
            Err(LocationError::BadEscape { at: prefix + 1 })
        );
        assert_eq!(
            decode("file://localhost/C:/a%"),
            Err(LocationError::BadEscape { at: prefix + 1 })
        );
    }

    #[test]
    fn an_encoded_nul_is_an_error() {
        assert_eq!(
            decode("file://localhost/C:/a%00.mp3"),
            Err(LocationError::NulByte)
        );
        assert_eq!(
            decode("file://localhost/C:/a\0.mp3"),
            Err(LocationError::NulByte)
        );
    }

    #[test]
    fn the_prefix_is_matched_ignoring_case_and_accepts_an_empty_host() {
        assert_eq!(drive("FILE://LOCALHOST/C:/a.mp3"), "C:/a.mp3");
        assert_eq!(drive("File://LocalHost/c:/a.mp3"), "c:/a.mp3");
        assert_eq!(drive("file:///C:/a.mp3"), "C:/a.mp3");
    }

    #[test]
    fn backslashes_in_a_windows_path_become_forward_slashes() {
        assert_eq!(
            drive("file://localhost/C:%5cMusic%5ca.mp3"),
            "C:/Music/a.mp3"
        );
        assert_eq!(drive("file://localhost/C:\\Music\\a.mp3"), "C:/Music/a.mp3");
    }

    #[test]
    fn a_drive_letter_in_the_host_position_is_a_drive_path_not_a_server() {
        assert_eq!(drive("file://C:/Music/a%20b.mp3"), "C:/Music/a b.mp3");
        assert_eq!(drive("file://d:/a.mp3"), "d:/a.mp3");
    }

    #[test]
    fn a_drive_root_is_a_drive_path() {
        assert_eq!(drive("file://localhost/D:/"), "D:/");
        assert_eq!(drive("file://localhost/D:"), "D:");
    }

    #[test]
    fn network_paths_decode_in_every_spelling() {
        for raw in [
            "file://localhost//nas/share/Music/a%20b.mp3",
            "file:////nas/share/Music/a%20b.mp3",
            "file://nas/share/Music/a%20b.mp3",
            "file://localhost/%5c%5cnas%5cshare%5cMusic%5ca%20b.mp3",
        ] {
            let path = file(raw);
            assert_eq!(path.style(), PathStyle::WindowsUnc, "{raw:?}");
            assert_eq!(path.as_str(), "//nas/share/Music/a b.mp3", "{raw:?}");
            assert_eq!(
                path.to_windows().as_deref(),
                Some("\\\\nas\\share\\Music\\a b.mp3")
            );
        }
    }

    #[test]
    fn a_network_path_needs_a_server_and_a_share() {
        assert_eq!(
            decode("file://localhost//nas"),
            Err(LocationError::BadNetworkPath)
        );
        assert_eq!(
            decode("file://localhost///share/a.mp3"),
            Err(LocationError::BadNetworkPath)
        );
        assert_eq!(decode("file://nas/"), Err(LocationError::BadNetworkPath));
    }

    #[test]
    fn mac_paths_decode_as_posix_paths_and_keep_their_case() {
        let path = file("file://localhost/Users/someone/Music/Caf%C3%A9%20#2.aiff");
        assert_eq!(path.style(), PathStyle::Posix);
        assert_eq!(path.as_str(), "/Users/someone/Music/Café #2.aiff");
        assert_eq!(path.to_windows(), None);
        let volume = file("file://localhost/Volumes/USB%20STICK/Contents/a.mp3");
        assert_eq!(volume.as_str(), "/Volumes/USB STICK/Contents/a.mp3");
        assert_ne!(
            file("file:///Users/A.mp3").match_key(),
            file("file:///Users/a.mp3").match_key()
        );
    }

    #[test]
    fn streaming_entries_are_streaming_not_errors() {
        for (raw, scheme, id) in [
            ("soundcloud:tracks:12345", "soundcloud", "tracks:12345"),
            (
                "spotify:track:1aBcDeFgHiJkLmNoPqRsTu",
                "spotify",
                "track:1aBcDeFgHiJkLmNoPqRsTu",
            ),
            ("tidal:tracks:77", "tidal", "tracks:77"),
            ("SoundCloud:tracks:1", "soundcloud", "tracks:1"),
            (
                "file://localhost/soundcloud:tracks:12345",
                "soundcloud",
                "tracks:12345",
            ),
            ("file://localhost/beatport:track:9", "beatport", "track:9"),
        ] {
            assert_eq!(
                decode(raw),
                Ok(Location::Streaming(StreamingId {
                    scheme: scheme.into(),
                    id: id.into(),
                })),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn values_that_are_neither_files_nor_streaming_are_errors() {
        assert_eq!(decode(""), Err(LocationError::Empty));
        assert_eq!(decode("no scheme here"), Err(LocationError::NoScheme));
        assert_eq!(decode(":empty"), Err(LocationError::NoScheme));
        assert_eq!(decode("1abc:x"), Err(LocationError::NoScheme));
        assert_eq!(decode("C:/Music/a.mp3"), Err(LocationError::BarePath));
        assert_eq!(decode("file:/C:/a.mp3"), Err(LocationError::BadFilePrefix));
        assert_eq!(decode("file:C:/a.mp3"), Err(LocationError::BadFilePrefix));
        assert_eq!(decode("file://localhost"), Err(LocationError::NoPath));
        assert_eq!(decode("file://localhost/"), Err(LocationError::NoPath));
    }

    #[test]
    fn file_name_and_components_skip_the_root() {
        let path = file("file://localhost/C:/Music/Sub/a%20b.mp3");
        assert_eq!(path.file_name(), "a b.mp3");
        assert_eq!(
            path.components().collect::<Vec<_>>(),
            ["Music", "Sub", "a b.mp3"]
        );
        let unc = file("file://localhost//nas/share/Music/a.mp3");
        assert_eq!(unc.components().collect::<Vec<_>>(), ["Music", "a.mp3"]);
        let mac = file("file:///Users/x/a.mp3");
        assert_eq!(
            mac.components().collect::<Vec<_>>(),
            ["Users", "x", "a.mp3"]
        );
    }

    #[test]
    fn windows_match_keys_ignore_letter_case_and_unicode_normal_form() {
        // Café spelled with a precomposed é, and with e + combining accent.
        let nfc = file("file://localhost/C:/Music/Caf%C3%A9.mp3");
        let nfd = file("file://localhost/c:/MUSIC/CAFE%CC%81.MP3");
        assert_ne!(nfc.as_str(), nfd.as_str());
        assert_eq!(nfc.match_key(), nfd.match_key());
        // NTFS doesn't expand ß to SS.
        assert_ne!(
            file("file:///C:/stra%C3%9Fe.mp3").match_key(),
            file("file:///C:/STRASSE.mp3").match_key()
        );
    }

    #[test]
    fn a_rekordbox_written_and_a_fully_encoded_location_match() {
        // rekordbox's own spelling (raw `#`, `(`, `,`, `+`, lowercase hex)
        // and our writer's (everything encoded, uppercase hex).
        let rekordbox = "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20#1%20(100%25%20Flip).wav";
        let ours = "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20%231%20%28100%25%20Flip%29.wav";
        assert_eq!(file(rekordbox), file(ours));
        assert_eq!(
            file(rekordbox).as_str(),
            "C:/Kit/tracks/04 Kit & Kin - Low Tide #1 (100% Flip).wav"
        );
    }

    // --- property tests ---

    /// Characters rekordbox 7.2 leaves raw (seen in its exports).
    const RB_RAW: &[u8] = b"-._~/:(),+#!";

    /// Encodes like rekordbox: lowercase hex, `# ( ) , + !` raw.
    fn encode_rekordbox_style(path: &str) -> String {
        encode(
            path,
            |b| b.is_ascii_alphanumeric() || RB_RAW.contains(&b),
            false,
        )
    }

    /// Encodes like our writer will (ROADMAP 1.9 rule 5): everything but
    /// unreserved characters and `/` `:`, uppercase hex.
    fn encode_fully(path: &str) -> String {
        encode(
            path,
            |b| b.is_ascii_alphanumeric() || b"-._~/:".contains(&b),
            true,
        )
    }

    fn encode(path: &str, raw: impl Fn(u8) -> bool, upper: bool) -> String {
        let mut out = String::from("file://localhost/");
        for b in path.bytes() {
            if raw(b) {
                out.push(b as char);
            } else if upper {
                out.push_str(&format!("%{b:02X}"));
            } else {
                out.push_str(&format!("%{b:02x}"));
            }
        }
        out
    }

    /// One file or folder name: any characters Windows allows, including
    /// the troublesome ones, and non-ASCII from every UTF-8 length.
    fn name() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                4 => proptest::char::range('a', 'z'),
                2 => proptest::sample::select(vec![
                    ' ', '#', '%', '+', '&', '\'', '(', ')', ',', '!', '[', ']', ';', '=', '@',
                    '$', '~', '.', '-', '_', '{', '}', '^', '`',
                ]),
                1 => proptest::char::range('\u{a0}', '\u{7ff}'),
                1 => proptest::char::range('\u{800}', '\u{d7ff}'),
                1 => proptest::char::range('\u{1f300}', '\u{1faff}'),
            ],
            1..12,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    fn windows_path() -> impl Strategy<Value = String> {
        (
            proptest::char::range('A', 'Z'),
            proptest::collection::vec(name(), 1..6),
        )
            .prop_map(|(letter, names)| format!("{letter}:/{}", names.join("/")))
    }

    proptest! {
        #[test]
        fn any_windows_path_survives_rekordbox_style_encoding(path in windows_path()) {
            let decoded = decode(&encode_rekordbox_style(&path));
            prop_assert_eq!(
                decoded,
                Ok(Location::File(FilePath { style: PathStyle::WindowsDrive, path: path.clone() }))
            );
        }

        #[test]
        fn any_windows_path_survives_full_encoding(path in windows_path()) {
            let decoded = decode(&encode_fully(&path));
            prop_assert_eq!(
                decoded,
                Ok(Location::File(FilePath { style: PathStyle::WindowsDrive, path: path.clone() }))
            );
        }

        #[test]
        fn hex_letter_case_never_changes_the_decoded_path(path in windows_path()) {
            prop_assert_eq!(
                decode(&encode_rekordbox_style(&path)),
                decode(&encode_fully(&path))
            );
        }

        #[test]
        fn any_mac_path_survives_encoding(names in proptest::collection::vec(name(), 1..6)) {
            let path = format!("/Users/{}", names.join("/"));
            let encoded = encode_rekordbox_style(&path[1..]);
            prop_assert_eq!(
                decode(&encoded),
                Ok(Location::File(FilePath { style: PathStyle::Posix, path: path.clone() }))
            );
        }

        #[test]
        fn any_network_path_survives_encoding(
            server in "[a-z][a-z0-9-]{0,10}",
            share in name(),
            names in proptest::collection::vec(name(), 1..4),
        ) {
            let path = format!("//{server}/{share}/{}", names.join("/"));
            let decoded = decode(&encode_fully(&path[1..]));
            prop_assert_eq!(
                decoded,
                Ok(Location::File(FilePath { style: PathStyle::WindowsUnc, path: path.clone() }))
            );
        }

        #[test]
        fn arbitrary_text_never_panics_the_decoder(raw in any::<String>()) {
            let _ = decode(&raw);
            let _ = decode(&format!("file://localhost/{raw}"));
        }

        #[test]
        fn arbitrary_escapes_never_panic_the_decoder(
            bytes in proptest::collection::vec(any::<u8>(), 0..40),
        ) {
            let raw: String = bytes.iter().map(|b| format!("%{b:02x}")).collect();
            let result = decode(&format!("file://localhost/C:/{raw}"));
            if let Ok(Location::File(path)) = &result {
                prop_assert_eq!(path.as_str().len(), 3 + bytes.len());
            }
        }
    }
}
