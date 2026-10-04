//! The commands the frontend can call, and the TypeScript bindings generated
//! from them by `tauri-specta` (ROADMAP §0.1).
//!
//! The bindings file is generated, never hand-edited. Regenerate it with
//! `cargo run --example export_bindings` from `src-tauri/`.

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;
use specta_typescript::Typescript;
use tauri::Runtime;
use tauri_specta::{collect_commands, collect_events, Builder};

mod error;
#[cfg(test)]
pub(crate) mod testing;

pub use error::{error_keys, ErrorKind, ErrorParam, IpcError};

/// Where the generated TypeScript bindings live, relative to `src-tauri/`.
pub const BINDINGS_REL_PATH: &str = "../src/bindings.ts";

/// The absolute path of the generated bindings in this checkout.
pub fn bindings_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(BINDINGS_REL_PATH)
}

/// The app's name, version and bundle ID, as the frontend sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
}

/// Returns the app's name, version and bundle ID.
// The sample command that proves a Rust command reaches TypeScript fully
// typed. It isn't generic over the runtime, so the same command list serves
// the real app and the mock runtime in tests.
#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME").to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        identifier: crate::IDENTIFIER.to_owned(),
    }
}

/// Every command the frontend can call, and every event it can hear. The
/// app's invoke handler, its event registry and the bindings export all come
/// from these lists, so they can't drift apart.
///
/// A command that can fail returns `Result<T, IpcError>`, never error text
/// (see `ipc/error.rs`).
//
// i64 and u64 values cross IPC as a TS `number`, which is exact up to 2^53:
// plenty for row ids and counters. Anything that can go past that (ns
// timestamps, hashes) must cross as a string or be converted first (e.g. to
// ms). tauri-specta's name for this setting is "dangerously" cast.
//
// One command or event per line, trailing comma, and rustfmt kept off, so
// lanes that add them each add a line and don't conflict.
#[rustfmt::skip]
pub fn specta_builder<R: Runtime>() -> Builder<R> {
    Builder::<R>::new()
        .dangerously_cast_bigints_to_number()
        .commands(collect_commands![
            app_info,
            crate::settings::key_notation,
            crate::settings::set_key_notation,
            crate::jobs::commands::activity,
            crate::jobs::commands::cancel_job,
            crate::ops::undo_last_operation,
            crate::ops::next_undo_operation,
            crate::scan::folders::music_folders,
            crate::scan::folders::add_music_folder,
            crate::scan::folders::remove_music_folder,
            crate::scan::scan_music_folders,
            crate::scan::watch::set_music_folder_watch,
            crate::rekordbox::source::rekordbox_xml_source,
            crate::rekordbox::source::read_rekordbox_xml,
            crate::rekordbox::source::set_rekordbox_xml_watch,
            crate::scan::online_only::read_online_only_files,
            crate::scan::online_only::set_read_online_only_files,
            crate::hash::hash_music_folders,
            crate::read::read_files,
            crate::fingerprint::fingerprint_files,
            crate::fingerprint::fingerprint_first,
            crate::relink::job::relink_rekordbox_tracks,
            crate::grouping::group_files,
            crate::missing::missing_tracks,
            crate::library::library_tracks,
            crate::library::promote_track,
            crate::library::remove_library_track,
            crate::start::rekordbox_offer,
            crate::start::add_rekordbox_tracks,
            crate::start::undo_add_rekordbox_tracks,
            crate::start::rekordbox_playlists,
            crate::all_music::all_music_tracks,
            crate::send::send_state,
            crate::send::prepare_send,
            crate::send::write_send,
            crate::after_send::commands::after_send_lists,
            crate::crates::create_crate,
            crate::crates::rename_crate,
            crate::crates::delete_crate,
            crate::crates::add_tracks_to_crate,
            crate::crates::remove_tracks_from_crate,
            crate::crates::list_crates,
            crate::crates::crate_tracks,
        ])
        .events(collect_events![
            crate::jobs::JobUpdates,
            crate::scan::ScannedFiles,
            crate::scan::VolumesChanged,
        ])
        // Every key notation's 24 names, so the frontend formats keys from the
        // same tables as the backend (tags/key.rs).
        .constant("KEY_NAMES", crate::tags::key::key_names())
        // Each error kind's i18n key, so the frontend looks up error
        // messages with keys TypeScript checks (src/api/errors.ts).
        .constant("ERROR_KEYS", error_keys())
        .typ::<crate::suggest::Suggestion<()>>()
}

/// Writes the TypeScript bindings to `path`. Needs no running app or window,
/// so CI can run it headless. The output depends only on the command list.
pub fn export_bindings(path: &Path) -> Result<(), specta_typescript::Error> {
    let header = "// Regenerate with `cargo run --example export_bindings` in src-tauri.\n";
    specta_builder::<tauri::Wry>().export(Typescript::default().header(header), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_export_is_byte_identical_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.ts"), dir.path().join("b.ts"));
        export_bindings(&a).unwrap();
        export_bindings(&b).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    }

    #[test]
    fn bindings_declare_the_app_info_command_and_its_typed_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bindings.ts");
        export_bindings(&path).unwrap();
        let ts = std::fs::read_to_string(&path).unwrap();
        assert!(
            ts.contains(r#"appInfo: () => __TAURI_INVOKE<AppInfo>("app_info")"#),
            "{ts}"
        );
        assert!(ts.contains("export type AppInfo = {"), "{ts}");
        for field in ["name: string", "version: string", "identifier: string"] {
            assert!(ts.contains(field), "missing `{field}` in:\n{ts}");
        }
    }

    #[test]
    fn bindings_declare_the_job_commands_and_the_job_updates_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.ts");
        export_bindings(&path).unwrap();
        let ts = std::fs::read_to_string(&path).unwrap();
        for expected in [
            r#"("activity")"#,
            r#"("cancel_job", { id })"#,
            r#"jobUpdates: makeEvent<JobUpdates>("job-updates")"#,
            "export type JobUpdates = JobUpdate[];",
            "export type JobUpdate = {",
            "kind: JobKind",
            "seq: number,",
            "priority: number,",
            "export type JobId = number;",
        ] {
            assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
        }
        // The string unions, one variant per line with its doc comment.
        for (name, variants) in [
            (
                "JobKind",
                &[
                    "scan",
                    "read",
                    "hash",
                    "fingerprint",
                    "group",
                    "analyze",
                    "embed",
                    "convert",
                    "export",
                    "read_rekordbox",
                    "relink",
                    "attach",
                    "quality",
                    "match",
                ][..],
            ),
            (
                "JobStatus",
                &["queued", "running", "done", "failed", "cancelled"][..],
            ),
            (
                "CancelOutcome",
                &["cancelled", "stopping", "not_active"][..],
            ),
        ] {
            let start = ts
                .find(&format!("export type {name} ="))
                .unwrap_or_else(|| panic!("no type {name} in:\n{ts}"));
            let union = &ts[start..start + ts[start..].find(';').unwrap()];
            let found: Vec<_> = union
                .lines()
                .filter_map(|l| l.trim().strip_prefix('"')?.split('"').next())
                .collect();
            assert_eq!(found, variants, "{name}:\n{union}");
        }
    }

    #[test]
    fn bindings_say_they_are_generated_and_how_to_regenerate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("header.ts");
        export_bindings(&path).unwrap();
        let ts = std::fs::read_to_string(&path).unwrap();
        assert!(
            ts.starts_with("// Regenerate with `cargo run --example export_bindings`"),
            "{ts}"
        );
        assert!(ts.contains("Do not edit this file manually"), "{ts}");
    }

    #[test]
    fn bindings_path_points_at_the_frontend_src_folder() {
        let path = bindings_path();
        assert!(
            path.ends_with("src/bindings.ts") || path.ends_with(r"src\bindings.ts"),
            "{path:?}"
        );
        let src = path.parent().unwrap();
        assert!(
            src.join("main.tsx").is_file(),
            "{src:?} is not the frontend src folder"
        );
    }

    #[test]
    fn bindings_declare_the_key_notation_commands_type_and_name_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.ts");
        export_bindings(&path).unwrap();
        let ts = std::fs::read_to_string(&path).unwrap();
        for expected in [
            r#"keyNotation: () => typedError<KeyNotation, IpcError>(__TAURI_INVOKE("key_notation"))"#,
            r#"setKeyNotation: (notation: KeyNotation) => typedError<null, IpcError>(__TAURI_INVOKE("set_key_notation", { notation }))"#,
            "export type KeyNotation = ",
            r#""camelot" | "#,
            r#""musical_standard" | "#,
            r#""musical_rekordbox" | "#,
            r#""musical_sharps" | "#,
            r#""musical_flats";"#,
            "export const KEY_NAMES = [",
            r#"{"names":["G#m","Ebm","Bbm","Fm","Cm","Gm","Dm","Am","Em","Bm","F#m","C#m","B","F#","Db","Ab","Eb","Bb","F","C","G","D","A","E"],"notation":"musical_standard"}"#,
        ] {
            assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
        }
    }

    #[test]
    fn app_info_reports_tracklist_pro_identity() {
        let info = app_info();
        assert_eq!(info.name, "tracklist-pro");
        assert_eq!(info.identifier, crate::IDENTIFIER);
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
