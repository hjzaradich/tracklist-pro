//! Writing a `Location` (ROADMAP 1.9 rule 5): the inverse of the reader's
//! decoding ([`crate::rekordbox::location`]).
//!
//! rekordbox's own exports leave `# ( ) , + !` raw and write lowercase
//! `%xx`. The writer encodes everything except ASCII letters, digits and
//! `- . _ ~ / :`, byte by byte over the path's UTF-8, with uppercase hex:
//! the form the rekordbox behavior check sent and rekordbox matched to its
//! own tracks without making duplicates (§5.2).
//!
//! Whatever is encoded is decoded again with the reader before it's
//! returned, and refused unless it reads back as the same path, so a
//! `Location` the app can't read itself is never sent.
//!
//! **A track rekordbox already has** is re-encoded only when its
//! `Location` is a drive path in rekordbox's usual shape: it starts with
//! the literal `file://localhost/C:/` (any drive letter), and escapes only
//! characters rekordbox itself escapes. That's the case the behavior check
//! sent and rekordbox updated in place. Anything else (a network or macOS
//! path, a server or nothing where `localhost` goes, backslashes, an
//! uppercase prefix, an escaped `/`, `:`, `.` or letter) is sent back
//! exactly as rekordbox wrote it, since re-spelling those is unverified.

use crate::rekordbox::location::{decode, FilePath, Location, PathStyle};

/// What every file `Location` starts with.
const PREFIX: &str = "file://localhost";

/// Why a path or a rekordbox `Location` can't be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationProblem {
    /// rekordbox's value doesn't decode.
    Undecodable,
    /// rekordbox's value is a streaming entry, which has no file.
    NotAFile,
    /// The path isn't a full Windows path: `C:\…` or `\\server\share\…`.
    NotAFullPath,
    /// The encoded value doesn't read back as the same path.
    DoesNotReadBack,
}

/// The `Location` for a path the reader decoded.
fn encode(path: &FilePath) -> String {
    let mut out = String::with_capacity(PREFIX.len() + path.as_str().len() * 2);
    out.push_str(PREFIX);
    // A drive path is `C:/…` and takes the `/` after the host; network
    // (`//server/…`) and macOS (`/Users/…`) paths bring their own.
    if path.style() == PathStyle::WindowsDrive {
        out.push('/');
    }
    for byte in path.as_str().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// The `Location` to send for a track rekordbox knows, from rekordbox's
/// own. A drive path in rekordbox's usual shape is re-encoded in the
/// writer's form, which names the same path, character for character;
/// any other shape is returned exactly as rekordbox wrote it.
pub fn from_rekordbox(raw: &str) -> Result<String, LocationProblem> {
    let path = match decode(raw) {
        Ok(Location::File(path)) => path,
        Ok(Location::Streaming(_)) => return Err(LocationProblem::NotAFile),
        Err(_) => return Err(LocationProblem::Undecodable),
    };
    let encoded = encode(&path);
    // The usual shape, and only escaping may differ: with every `%xx`
    // resolved, the two are the same text.
    let same_shape = is_usual_drive_location(raw)
        && path.style() == PathStyle::WindowsDrive
        && matches!((unescaped(raw), unescaped(&encoded)), (Some(a), Some(b)) if a == b);
    if !same_shape {
        return Ok(raw.to_owned());
    }
    if reads_back_as(&encoded, &path) {
        Ok(encoded)
    } else {
        Err(LocationProblem::DoesNotReadBack)
    }
}

/// Characters rekordbox 7 leaves raw in a `Location`, besides ASCII
/// letters and digits (seen in its exports, §5.3). It never writes one of
/// these as `%xx`.
const REKORDBOX_RAW: &[u8] = b"-._~/:(),+#!";

/// Whether `raw` is spelled the way rekordbox spells a drive path: the
/// literal `file://localhost/`, a drive letter, `:/`, and no escape of a
/// character rekordbox leaves raw (`%2F`, `%3A`, `%2E`, `%41`…).
fn is_usual_drive_location(raw: &str) -> bool {
    let Some(rest) = raw.strip_prefix("file://localhost/") else {
        return false;
    };
    let b = rest.as_bytes();
    if !(b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/') {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let escaped = std::str::from_utf8(b.get(i + 1..i + 3).unwrap_or_default())
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            match escaped {
                Some(byte) if !(byte.is_ascii_alphanumeric() || REKORDBOX_RAW.contains(&byte)) => {}
                _ => return false,
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    true
}

/// Whether the reader decodes `encoded` to exactly `path`. For anything
/// [`encode`] makes it does (the property tests show it), so this guards
/// against a later change to either side rather than a known case.
fn reads_back_as(encoded: &str, path: &FilePath) -> bool {
    matches!(decode(encoded), Ok(Location::File(back)) if back == *path)
}

/// `value` with each `%xx` replaced by its byte; `None` if a `%` isn't
/// followed by two hex digits.
fn unescaped(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The `Location` for a file's full Windows path (`C:\Music\a.mp3`,
/// `\\server\share\a.mp3`), for a track rekordbox doesn't know yet.
pub fn from_windows_path(path: &str) -> Result<String, LocationProblem> {
    let b = path.as_bytes();
    let drive = b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
    // `\\?\…` and `\\.\…` are device forms, not a server.
    let network = path
        .strip_prefix(r"\\")
        .is_some_and(|rest| !rest.starts_with(['?', '.', '\\']));
    if !(drive || network) || path.contains('/') {
        return Err(LocationProblem::NotAFullPath);
    }
    let slashed = path.replace('\\', "/");
    let lead = if drive { "/" } else { "" };
    // The reader builds the path value; nothing here does.
    let Ok(Location::File(as_read)) =
        decode(&format!("{PREFIX}{lead}{}", escape_percent(&slashed)))
    else {
        return Err(LocationProblem::NotAFullPath);
    };
    let style = if drive {
        PathStyle::WindowsDrive
    } else {
        PathStyle::WindowsUnc
    };
    if as_read.style() != style || as_read.as_str() != slashed {
        return Err(LocationProblem::NotAFullPath);
    }
    let encoded = encode(&as_read);
    match decode(&encoded) {
        Ok(Location::File(back)) if back.to_windows().as_deref() == Some(path) => Ok(encoded),
        _ => Err(LocationProblem::DoesNotReadBack),
    }
}

/// `%` as `%25`, so the reader takes every other character as itself.
fn escape_percent(path: &str) -> String {
    path.replace('%', "%25")
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn read(location: &str) -> FilePath {
        match decode(location) {
            Ok(Location::File(path)) => path,
            other => panic!("{location}: {other:?}"),
        }
    }

    #[test]
    fn a_drive_path_is_fully_percent_encoded_with_uppercase_hex() {
        assert_eq!(
            from_windows_path(r"C:\Kit\04 Kit & Kin - Low Tide #1 (100% Flip).wav").unwrap(),
            "file://localhost/C:/Kit/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20%231%20%28100%25%20Flip%29.wav"
        );
    }

    #[test]
    fn unicode_is_encoded_byte_by_byte_as_utf8() {
        assert_eq!(
            from_windows_path("D:\\Café 日本 🚀.m4a").unwrap(),
            "file://localhost/D:/Caf%C3%A9%20%E6%97%A5%E6%9C%AC%20%F0%9F%9A%80.m4a"
        );
    }

    #[test]
    fn only_letters_digits_and_six_marks_are_left_raw() {
        let encoded = from_windows_path(r"e:\aZ09-._~\!$&'()+,;=@[]^`{} #%.mp3").unwrap();
        assert_eq!(
            encoded,
            "file://localhost/e:/aZ09-._~/%21%24%26%27%28%29%2B%2C%3B%3D%40%5B%5D%5E%60%7B%7D%20%23%25.mp3"
        );
    }

    #[test]
    fn a_network_path_keeps_its_server_and_share() {
        let encoded = from_windows_path(r"\\nas\Music Share\a b.flac").unwrap();
        assert_eq!(encoded, "file://localhost//nas/Music%20Share/a%20b.flac");
        assert_eq!(
            read(&encoded).to_windows().unwrap(),
            r"\\nas\Music Share\a b.flac"
        );
    }

    #[test]
    fn awkward_paths_read_back_through_the_reader_unchanged() {
        for path in [
            r"C:\Music\a.mp3",
            r"C:\Music\50% off.mp3",
            r"C:\Music\%41 literal.mp3",
            r"C:\Music\Rock & Roll #1.mp3",
            r"C:\Music\A + B.mp3",
            r"C:\Music\it's (live), really!.mp3",
            "C:\\Music\\e\u{301} decomposed.mp3",
            "C:\\Music\\é composed.mp3",
            "z:\\日本語\\トラック.aiff",
            "C:\\Music\\🚀🎧.wav",
            r"C:\Music\trailing dot.\a.mp3",
            r"C:\Music\ leading space.mp3",
            r"C:\Music\a?b.mp3",
            r"\\server\share\x.mp3",
        ] {
            let encoded = from_windows_path(path).unwrap();
            assert_eq!(
                read(&encoded).to_windows().as_deref(),
                Some(path),
                "{encoded}"
            );
            assert!(
                encoded.is_ascii() && !encoded.contains(['#', ' ', '&', '+']),
                "{encoded}"
            );
        }
    }

    #[test]
    fn a_path_that_isnt_a_full_windows_path_is_refused() {
        for path in [
            "",
            "a.mp3",
            r"Music\a.mp3",
            r"C:",
            r"C:\",
            r"C:a.mp3",
            "C:/Music/a.mp3",
            r"C:\Music/a.mp3",
            r"\\?\C:\Music\a.mp3",
            r"\\.\C:\Music\a.mp3",
            r"\\server",
            r"\\server\",
            r"\\\server\share\a.mp3",
            "/Users/someone/a.mp3",
            r"\Music\a.mp3",
            "soundcloud:tracks:1",
        ] {
            assert_eq!(
                from_windows_path(path),
                Err(LocationProblem::NotAFullPath),
                "{path:?}"
            );
        }
        assert!(from_windows_path("C:\\Music\\a\0b.mp3").is_err());
    }

    #[test]
    fn rekordboxs_own_location_is_reencoded_to_the_same_path() {
        let rekordbox = "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20#1%20(100%25%20Flip).wav";
        let ours = from_rekordbox(rekordbox).unwrap();
        assert_eq!(
            ours,
            "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20%231%20%28100%25%20Flip%29.wav"
        );
        assert_eq!(read(&ours), read(rekordbox));
        // Lowercase hex and raw `, +` come back uppercase and encoded.
        let rekordbox = "file://localhost/C:/Kit/North%20Wind,%20Late%20%f0%9f%9a%80%20Caf%c3%a9%20A%20+%20B%20%5bx%5d.m4a";
        let ours = from_rekordbox(rekordbox).unwrap();
        assert_eq!(read(&ours), read(rekordbox));
        assert!(ours.contains("%F0%9F%9A%80") && ours.contains("%2C") && ours.contains("%2B"));
        // Already in the writer's form: unchanged.
        assert_eq!(from_rekordbox(&ours).unwrap(), ours);
    }

    #[test]
    fn a_known_location_in_any_other_shape_is_sent_back_exactly_as_rekordbox_wrote_it() {
        for raw in [
            // Network and macOS paths, in rekordbox's own escaping.
            "file://localhost//server/share/Music/a%20b%20#1%20(x),%20caf%c3%a9.mp3",
            "file://localhost/Users/someone/Music/a%20b%20#1%20caf%c3%a9.mp3",
            // A server, a drive or nothing where `localhost` goes.
            "file://server/share/a%20b%20#1.mp3",
            "file://C:/Kit/a%20b%20#1.mp3",
            "file:///C:/Kit/a%20b%20#1.mp3",
            // An uppercase prefix or host.
            "FILE://localhost/C:/Kit/a%20b%20#1.mp3",
            "file://LOCALHOST/C:/Kit/a%20b%20#1.mp3",
            // Escapes rekordbox never writes: a separator, the drive's
            // colon or the drive letter, a dot segment, a plain letter, a
            // character it leaves raw.
            "file://localhost/C:/Kit%2fa%20b.mp3",
            "file://localhost/C:/Kit%2Fa%20b.mp3",
            "file://localhost/C%3a/Kit/a%20b.mp3",
            "file://localhost/%43:/Kit/a%20b.mp3",
            "file://localhost/C:%2fKit/a%20b.mp3",
            "file://localhost/C:/Kit/%2e%2e/a%20b.mp3",
            "file://localhost/C:/Kit/%2e/a%20b.mp3",
            "file://localhost/C:/Kit/%41%20b.mp3",
            "file://localhost/C:/Kit/a%20b%231.mp3",
            "file://localhost/C:/Kit/a%20%28b%29.mp3",
            "file://localhost/C:/Kit/a%2c%20b.mp3",
            "file://localhost/C:/Kit/a%20b%2emp3",
            // No path after the drive.
            "file://localhost/C:",
            // Backslashes, raw or escaped.
            "file://localhost/C:\\Kit\\a%20b%20#1.mp3",
            "file://localhost/C:/Kit%5ca%20b%20#1.mp3",
            "file://localhost/\\\\server\\share\\a%20b.mp3",
        ] {
            assert_eq!(from_rekordbox(raw).as_deref(), Ok(raw), "{raw}");
        }
    }

    #[test]
    fn a_usual_drive_location_is_reencoded_whatever_its_escaping() {
        for (raw, ours) in [
            (
                "file://localhost/C:/Kit/a.mp3",
                "file://localhost/C:/Kit/a.mp3",
            ),
            (
                "file://localhost/c:/Kit/a b.mp3",
                "file://localhost/c:/Kit/a%20b.mp3",
            ),
            (
                "file://localhost/C:/Kit/caf\u{e9}.mp3",
                "file://localhost/C:/Kit/caf%C3%A9.mp3",
            ),
            (
                "file://localhost/C:/Kit/caf%c3%a9.mp3",
                "file://localhost/C:/Kit/caf%C3%A9.mp3",
            ),
            (
                "file://localhost/z:/Kit/a#!(b),+c.mp3",
                "file://localhost/z:/Kit/a%23%21%28b%29%2C%2Bc.mp3",
            ),
            (
                "file://localhost/C:/Kit/100%25%20%26%20%5bx%5D%27.mp3",
                "file://localhost/C:/Kit/100%25%20%26%20%5Bx%5D%27.mp3",
            ),
        ] {
            assert_eq!(from_rekordbox(raw).as_deref(), Ok(ours), "{raw}");
            assert_eq!(read(ours), read(raw));
        }
    }

    #[test]
    fn the_usual_shape_is_the_literal_prefix_a_drive_and_only_rekordboxs_escapes() {
        for usual in [
            "file://localhost/C:/a.mp3",
            "file://localhost/z:/Kit/a%20b%26%5b%c3%A9#(),+!.mp3",
        ] {
            assert!(is_usual_drive_location(usual), "{usual}");
        }
        for other in [
            "FILE://localhost/C:/a.mp3",
            "file://LOCALHOST/C:/a.mp3",
            "File://Localhost/C:/a.mp3",
            "file:///C:/a.mp3",
            "file://localhost//server/share/a.mp3",
            "file://localhost/Users/a.mp3",
            "file://localhost/C:",
            "file://localhost/C:a.mp3",
            "file://localhost/C:/a%2fb.mp3",
            "file://localhost/C:/a%3Ab.mp3",
            "file://localhost/C:/a%2Eb.mp3",
            "file://localhost/C:/a%62.mp3",
            "file://localhost/C:/a%7e.mp3",
            "file://localhost/C:/a%2.mp3",
            "file://localhost/C:/a%",
            "",
        ] {
            assert!(!is_usual_drive_location(other), "{other}");
        }
    }

    #[test]
    fn an_encoded_location_must_read_back_as_the_very_same_path() {
        let path = read("file://localhost/C:/Kit/a%20b.mp3");
        assert!(reads_back_as("file://localhost/C:/Kit/a%20b.mp3", &path));
        // Another path, another letter case, another kind, not a file,
        // not decodable.
        for other in [
            "file://localhost/C:/Kit/a%20c.mp3",
            "file://localhost/c:/kit/A%20B.mp3",
            "file://localhost//C:/Kit/a%20b.mp3",
            "soundcloud:tracks:1",
            "file://localhost/C:/Kit/a%2.mp3",
        ] {
            assert!(!reads_back_as(other, &path), "{other}");
        }
    }

    #[test]
    fn escapes_resolve_to_bytes_and_a_broken_one_to_nothing() {
        assert_eq!(unescaped("a%20b%C3%a9#").unwrap(), b"a b\xc3\xa9#");
        assert_eq!(unescaped("100%"), None);
        assert_eq!(unescaped("%2"), None);
        assert_eq!(unescaped("%zz"), None);
    }

    #[test]
    fn a_streaming_or_undecodable_location_from_rekordbox_is_refused() {
        assert_eq!(
            from_rekordbox("soundcloud:tracks:12345"),
            Err(LocationProblem::NotAFile)
        );
        assert_eq!(from_rekordbox(""), Err(LocationProblem::Undecodable));
        assert_eq!(
            from_rekordbox("file://localhost/C:/a%zz.mp3"),
            Err(LocationProblem::Undecodable)
        );
        assert_eq!(
            from_rekordbox("C:/Music/a.mp3"),
            Err(LocationProblem::Undecodable)
        );
    }

    /// One file or folder name: characters Windows allows, the awkward
    /// ones included, and non-ASCII of every UTF-8 length.
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

    proptest! {
        #[test]
        fn any_drive_path_reads_back_unchanged(
            drive in proptest::char::range('A', 'Z'),
            names in proptest::collection::vec(name(), 1..6),
        ) {
            let path = format!("{drive}:\\{}", names.join("\\"));
            let encoded = from_windows_path(&path).unwrap();
            prop_assert_eq!(read(&encoded).to_windows(), Some(path));
            prop_assert!(encoded.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~/:%".contains(&b)));
        }

        #[test]
        fn any_network_path_reads_back_unchanged(
            server in "[a-z][a-z0-9-]{0,8}",
            names in proptest::collection::vec(name(), 2..6),
        ) {
            let path = format!("\\\\{server}\\{}", names.join("\\"));
            let encoded = from_windows_path(&path).unwrap();
            prop_assert_eq!(read(&encoded).to_windows(), Some(path));
        }

        #[test]
        fn any_location_the_reader_decodes_is_reencoded_to_the_same_path_or_refused(
            raw in "file://localhost/[ -~]{0,40}",
        ) {
            if let Ok(ours) = from_rekordbox(&raw) {
                prop_assert_eq!(decode(&ours), decode(&raw));
                // Either rekordbox's own string, or one that differs from
                // it in escaping only.
                prop_assert!(ours == raw || unescaped(&ours) == unescaped(&raw));
            }
        }
    }
}
