//! Tests for stage 2: what one file read finds ([`files`]), and the read
//! job over real temp folders ([`job`], Windows only, like the walk's).

mod files;
#[cfg(windows)]
mod job;

use super::{read_job, scope};
use crate::jobs::{JobKind, Priority};
use crate::scan::folders::MusicFolderId;
use crate::scan_state::Scope;

#[test]
fn a_read_job_targets_its_music_folders_or_every_folder() {
    let all = read_job(None);
    assert_eq!(all.kind, JobKind::Read);
    assert_eq!(all.priority, Priority::NORMAL);
    assert_eq!(scope(all.target.as_ref()), Some(Scope::All));

    let some = read_job(Some(vec![MusicFolderId(3), MusicFolderId(7)]));
    assert_eq!(
        scope(some.target.as_ref()),
        Some(Scope::Folders(vec![3, 7]))
    );

    let bad = serde_json::json!({ "music_folder_ids": ["x"] });
    assert_eq!(scope(Some(&bad)), None);
    assert_eq!(scope(Some(&serde_json::json!({}))), None);
}
