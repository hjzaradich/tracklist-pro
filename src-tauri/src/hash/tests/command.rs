//! The command that starts a hash job, as the frontend sees it.

use crate::jobs::{JobKind, Priority};

#[test]
fn the_bindings_declare_hash_music_folders_with_optional_folder_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    crate::ipc::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();
    let expected = r#"hashMusicFolders: (ids: MusicFolderId[] | null) => typedError<JobId, IpcError>(__TAURI_INVOKE("hash_music_folders", { ids }))"#;
    assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
}

#[test]
fn a_hash_job_for_named_folders_targets_them_at_background_priority() {
    let job = crate::hash::hash_job(Some(vec![crate::scan::MusicFolderId(3)]));
    assert_eq!(job.kind, JobKind::Hash);
    assert_eq!(job.priority, Priority::BACKGROUND);
    assert_eq!(
        job.target,
        Some(serde_json::json!({ "music_folder_ids": [3] }))
    );
    assert_eq!(crate::hash::hash_job(None).target, None);
}
