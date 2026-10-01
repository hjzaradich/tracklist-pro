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

/// rekordbox's own `Location` for a track it knows, re-encoded in the
/// writer's form. It names the same path, character for character.
pub fn from_rekordbox(raw: &str) -> Result<String, LocationProblem> {
    let path = match decode(raw) {
        Ok(Location::File(path)) => path,
        Ok(Location::Streaming(_)) => return Err(LocationProblem::NotAFile),
        Err(_) => return Err(LocationProblem::Undecodable),
    };
    let encoded = encode(&path);
    match decode(&encoded) {
        Ok(Location::File(back)) if back == path => Ok(encoded),
        _ => Err(LocationProblem::DoesNotReadBack),
    }
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
    fn network_and_mac_locations_from_rekordbox_keep_their_shape() {
        for raw in [
            "file://localhost//server/share/Music/a%20b.mp3",
            "file://localhost/Users/someone/Music/a%20b.mp3",
        ] {
            let ours = from_rekordbox(raw).unwrap();
            assert_eq!(ours, raw);
            assert_eq!(read(&ours), read(raw));
        }
        // A server in the host position is written rekordbox's way.
        assert_eq!(
            from_rekordbox("file://server/share/a.mp3").unwrap(),
            "file://localhost//server/share/a.mp3"
        );
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
            }
        }
    }
}
