//! The best-file library's tests. Every name here is invented.
//!
//! - `steps`: one test per step of the order. Each proves that the step
//!   separates two files that are equal before it, that it holds against
//!   every later step, and that it is the reason named.
//! - `order`: the order is total and independent of input order; a flagged
//!   file never outranks an ok one; what each row's reason says.
//! - `quality`: the verdict classes, the cutoff's steps, the format lists,
//!   tag completeness.
//! - `place`: the place sub-steps; "on disk" is not "volume online".
//! - `pick`: the tag picking rule. `art`: the picture's.

use super::*;

mod art;
mod order;
mod pick;
mod place;
mod quality;
mod steps;

/// A file that is the best at every step. Every test changes only what it
/// is about.
pub(super) fn file(id: i64) -> FileFacts {
    FileFacts {
        file_id: id,
        on_disk: true,
        plays_in_dj_app: true,
        format: Format {
            codec: Some(Codec::Mp3),
            kbps: Some(320),
        },
        quality: Quality {
            verdict: Some(Verdict::Ok),
            cutoff_hz: Some(20_000),
            decode_errors: 0,
        },
        tags: TagFields::default(),
        art: None,
        rekordbox: RekordboxLink::Unknown,
        place: Place {
            volume_online: true,
            folder_role: FolderRole::Scan,
            volume_kind: VolumeKind::Internal,
            folder_order: 1,
        },
        chosen: false,
    }
}

/// A file changed in the one way a test is about.
pub(super) fn with(mut file: FileFacts, change: impl FnOnce(&mut FileFacts)) -> FileFacts {
    change(&mut file);
    file
}

/// Tags with a value in each of [`COMPLETENESS_FIELDS`].
pub(super) fn full_tags() -> TagFields {
    TagFields {
        title: Some("Velo".into()),
        artist: Some("Halden Rook".into()),
        album: Some("Nightdrive".into()),
        genre: Some("House".into()),
        label: Some("Kestrel Audio".into()),
        year: Some(2019),
        ..TagFields::default()
    }
}

pub(super) fn picture(bytes: u32) -> Art {
    Art {
        bytes,
        width: None,
        height: None,
        hash: None,
    }
}

pub(super) fn ids(ranking: &Ranking) -> Vec<i64> {
    ranking.order.iter().map(|row| row.file_id).collect()
}
