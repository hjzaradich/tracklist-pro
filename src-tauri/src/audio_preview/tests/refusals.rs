//! What is refused, and that each refusal has its own code and opens
//! nothing it didn't need to.

use std::fs;

use super::aiff::Made;
use super::support::Lib;
use crate::audio_preview::PreviewRefusal;
use crate::fingerprint::tests::audio::{pcm, wav};

#[test]
fn an_id_that_is_not_a_scanned_file_is_refused() {
    let lib = Lib::new();
    lib.put("real.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    // No such row.
    let got = lib.get(999, None);
    assert_eq!(got.status, 404);
    assert_eq!(got.refusal(), Some("unknown_file"));
    assert!(got.body.is_empty());
    // A file lying in the music folder that no scan has recorded has no id.
    fs::write(lib.path_of("unscanned.wav"), wav(&pcm(2, 0.5, 44_100, 1))).unwrap();
    assert_eq!(lib.get(2, None).refusal(), Some("unknown_file"));
}

#[test]
fn a_path_can_never_be_smuggled_in_the_url() {
    let lib = Lib::new();
    let id = lib.put("real.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    let secret = lib.outside.join("secret.wav");
    fs::write(&secret, wav(&pcm(3, 0.5, 44_100, 1))).unwrap();
    let absolute = secret.to_string_lossy().replace('\\', "/");
    let paths = [
        format!("/{absolute}"),
        format!("/{}", absolute.replace(':', "%3A")),
        "/../outside/secret.wav".to_owned(),
        "/%2e%2e/outside/secret.wav".to_owned(),
        "/music/real.wav".to_owned(),
        "/real.wav".to_owned(),
        format!("/{id}/../../outside/secret.wav"),
        format!("/{id}/real.wav"),
        format!("/{id}.wav"),
        format!("/{id}%2f..%2fsecret"),
        format!("//{id}"),
        "/".to_owned(),
        String::new(),
    ];
    for path in paths {
        let got = lib.ask(&path, None);
        assert_eq!(got.status, 400, "{path}");
        assert!(got.body.is_empty(), "{path}");
    }
    // The id alone works.
    assert_eq!(lib.get(id, None).status, 200);
}

#[test]
fn an_online_only_file_is_never_opened_even_when_the_user_allowed_reading_them() {
    let lib = Lib::new();
    // The row says online only and nothing is on disk: opening it would
    // answer "missing", so the code proves it was never opened.
    let id = lib.row("cloud.wav", true, true);
    let got = lib.get(id, None);
    assert_eq!(got.status, 403);
    assert_eq!(got.refusal(), Some("online_only"));

    // The scan's opt-in to read online-only files doesn't change that.
    lib.writer
        .call(|c| {
            c.execute(
                "INSERT INTO setting (key, value) VALUES ('read_online_only_files', json('true'))",
                [],
            )
        })
        .unwrap();
    assert_eq!(lib.get(id, None).refusal(), Some("online_only"));

    // And a file that is on disk and marked is not played either.
    let id = lib.put("cloud-too.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    lib.writer
        .call(move |c| c.execute("UPDATE file SET online_only = 1 WHERE id = ?1", [id]))
        .unwrap();
    assert_eq!(lib.get(id, None).refusal(), Some("online_only"));
}

/// Makes the file at `path` look online-only (a OneDrive placeholder) to the
/// attribute query, as the read stage's tests do: a real placeholder needs a
/// sync provider.
fn set_online_only(path: &std::path::Path, online_only: bool) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE, INVALID_FILE_ATTRIBUTES,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    assert_ne!(attributes, INVALID_FILE_ATTRIBUTES, "{}", path.display());
    let attributes = if online_only {
        attributes | FILE_ATTRIBUTE_OFFLINE
    } else {
        attributes & !FILE_ATTRIBUTE_OFFLINE
    };
    assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), attributes) }, 0);
}

#[test]
fn a_file_that_became_online_only_since_the_scan_is_refused_by_its_attributes() {
    // The row says nothing about OneDrive (the scan saw a normal file); the
    // file on disk is a placeholder now.
    let lib = Lib::new();
    let id = lib.put("became.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    assert_eq!(lib.get(id, None).status, 200);

    let path = lib.path_of("became.wav");
    set_online_only(&path, true);
    let got = lib.get(id, None);
    assert_eq!(got.status, 403);
    assert_eq!(got.refusal(), Some("online_only"));

    // The scan's opt-in to read online-only files doesn't change that.
    lib.writer
        .call(|c| {
            c.execute(
                "INSERT INTO setting (key, value) VALUES ('read_online_only_files', json('true'))",
                [],
            )
        })
        .unwrap();
    assert_eq!(lib.get(id, None).refusal(), Some("online_only"));

    // Available on this PC again, it plays.
    set_online_only(&path, false);
    assert_eq!(lib.get(id, None).status, 200);
}

#[test]
fn a_missing_file_says_missing() {
    let lib = Lib::new();
    // The last scan didn't find it.
    let gone = lib.put("gone.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    lib.writer
        .call(move |c| c.execute("UPDATE file SET present = 0 WHERE id = ?1", [gone]))
        .unwrap();
    let got = lib.get(gone, None);
    assert_eq!(got.status, 404);
    assert_eq!(got.refusal(), Some("missing"));

    // The scan found it, and it has been deleted since.
    let deleted = lib.put("deleted.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    fs::remove_file(lib.path_of("deleted.wav")).unwrap();
    assert_eq!(lib.get(deleted, None).refusal(), Some("missing"));
    // A folder where a file was.
    let id = lib.row("folder.wav", true, false);
    fs::create_dir(lib.path_of("folder.wav")).unwrap();
    assert_eq!(lib.get(id, None).refusal(), Some("missing"));
}

#[test]
fn a_file_on_an_unplugged_drive_says_the_drive_is_not_connected() {
    let lib = Lib::new();
    let id = lib.put("a.wav", &wav(&pcm(1, 0.5, 44_100, 1)));
    lib.volume.set_online(false);
    let got = lib.get(id, None);
    assert_eq!(got.status, 503);
    assert_eq!(got.refusal(), Some("drive_not_connected"));
    // Plugged in again, it plays.
    lib.volume.set_online(true);
    assert_eq!(lib.get(id, None).status, 200);
}

#[test]
fn a_file_that_cannot_be_decoded_says_so() {
    let lib = Lib::new();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("words.mp3", b"this is not audio at all".repeat(100)),
        ("empty.wav", Vec::new()),
        ("wma.mp3", {
            // An ASF header: a format the player can't play.
            let mut asf = vec![
                0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62,
                0xCE, 0x6C,
            ];
            asf.resize(2_000, 0);
            asf
        }),
        (
            "cut.aiff",
            Made::new(44_100, 2, 16, 1_000).bytes()[..30].to_vec(),
        ),
        ("three.aiff", Made::new(48_000, 3, 16, 1_000).bytes()),
    ];
    for (name, bytes) in cases {
        let id = lib.put(name, &bytes);
        let got = lib.get(id, None);
        assert_eq!(got.status, 415, "{name}");
        assert_eq!(got.refusal(), Some("not_decodable"), "{name}");
        assert!(got.body.is_empty(), "{name}");
    }
}

#[test]
fn each_refusal_has_its_own_code_the_player_can_read() {
    let all = [
        (PreviewRefusal::UnknownFile, "unknown_file"),
        (PreviewRefusal::Missing, "missing"),
        (PreviewRefusal::DriveNotConnected, "drive_not_connected"),
        (PreviewRefusal::OnlineOnly, "online_only"),
        (PreviewRefusal::Unreadable, "unreadable"),
        (PreviewRefusal::NotDecodable, "not_decodable"),
    ];
    for (refusal, code) in all {
        assert_eq!(refusal.code(), code);
        assert_eq!(
            serde_json::to_string(&refusal).unwrap(),
            format!("\"{code}\"")
        );
    }
    // Responses are distinct per code, too.
    let mut seen = std::collections::BTreeSet::new();
    for (refusal, _) in all {
        let reply = crate::audio_preview::serve::refused(refusal);
        let code = reply
            .headers()
            .get(crate::audio_preview::serve::REFUSAL_HEADER)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(seen.insert(code));
    }
}

#[test]
fn the_player_can_ask_whether_a_file_plays_and_get_the_code_when_it_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    crate::ipc::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();
    for expected in [
        "previewCheck: (fileId: number) => __TAURI_INVOKE<",
        r#"| null>("preview_check", { fileId })"#,
        "export type PreviewRefusal =",
        r#""unknown_file""#,
        r#""missing""#,
        r#""drive_not_connected""#,
        r#""online_only""#,
        r#""unreadable""#,
        r#""not_decodable";"#,
    ] {
        assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
    }
}
