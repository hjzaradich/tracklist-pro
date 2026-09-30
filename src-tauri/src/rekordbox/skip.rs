//! What isn't one of the user's tracks (ROADMAP 1.2, §5.3): deleted rows,
//! rekordbox's own demo tracks and samples, the "CUE Analysis Playlist",
//! and streaming entries. Every skip is counted by reason, and so is every
//! playlist entry that points at a skipped track, so nothing disappears
//! without a number to show for it.
//!
//! Streaming entries are the exception to "skipped": they stay in the
//! collection as tracks with no file ("streaming, no file"), and are
//! counted here so they aren't matched against files.

use super::collection::Track;
use super::location::{FilePath, Location};

/// Why a track isn't one of the user's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// Marked deleted (`rb_local_deleted`).
    Deleted,
    /// One of rekordbox's built-in demo tracks.
    DemoTrack,
    /// One of rekordbox's built-in sampler sounds.
    Sample,
    /// A streaming entry (SoundCloud, Spotify…): kept, but has no file.
    Streaming,
}

/// A count per [`SkipReason`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReasonCounts {
    pub deleted: usize,
    pub demo_tracks: usize,
    pub samples: usize,
    pub streaming: usize,
}

impl ReasonCounts {
    pub fn get(&self, reason: SkipReason) -> usize {
        match reason {
            SkipReason::Deleted => self.deleted,
            SkipReason::DemoTrack => self.demo_tracks,
            SkipReason::Sample => self.samples,
            SkipReason::Streaming => self.streaming,
        }
    }

    pub(crate) fn add(&mut self, reason: SkipReason) {
        match reason {
            SkipReason::Deleted => self.deleted += 1,
            SkipReason::DemoTrack => self.demo_tracks += 1,
            SkipReason::Sample => self.samples += 1,
            SkipReason::Streaming => self.streaming += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.deleted + self.demo_tracks + self.samples + self.streaming
    }
}

/// Everything skipped while reading one export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkipCounts {
    /// COLLECTION tracks, by reason.
    pub tracks: ReasonCounts,
    /// Playlist entries pointing at a skipped (or streaming) track, by the
    /// track's reason. Entries of the CUE Analysis Playlist aren't included.
    pub entries: ReasonCounts,
    /// CUE Analysis Playlists left out of the playlist tree.
    pub cue_analysis_playlists: usize,
    /// The entries those playlists held.
    pub cue_analysis_entries: usize,
}

/// The playlist rekordbox fills itself for background analysis.
pub const CUE_ANALYSIS_PLAYLIST: &str = "CUE Analysis Playlist";

/// Folders rekordbox installs its demo tracks and samples into, as path
/// components below the user's folder. Matched anywhere in a decoded path,
/// ignoring case, so the user name and drive don't matter.
const BUILT_IN: &[(&[&str], SkipReason)] = &[
    (
        &["music", "pioneerdj", "demo tracks"],
        SkipReason::DemoTrack,
    ),
    (&["music", "rekordbox", "sampler"], SkipReason::Sample),
];

/// Why `track` should be skipped, if it should.
pub fn reason(track: &Track) -> Option<SkipReason> {
    if is_deleted(track) {
        return Some(SkipReason::Deleted);
    }
    match &track.location {
        Ok(Location::Streaming(_)) => Some(SkipReason::Streaming),
        Ok(Location::File(path)) => built_in(path),
        Err(_) => None,
    }
}

/// `rb_local_deleted` set to anything but `0`. Unconfirmed in XML exports
/// (none seen so far; it may be a master.db column only), kept per §5.3.
fn is_deleted(track: &Track) -> bool {
    track
        .attrs
        .get("rb_local_deleted")
        .is_some_and(|v| !v.is_empty() && v != "0")
}

/// Whether the path is inside one of rekordbox's built-in folders.
pub fn built_in(path: &FilePath) -> Option<SkipReason> {
    let parts: Vec<String> = path.components().map(str::to_lowercase).collect();
    BUILT_IN.iter().find_map(|(folders, reason)| {
        // The file must be inside the folder, not be it.
        (parts.len() > folders.len()
            && parts[..parts.len() - 1]
                .windows(folders.len())
                .any(|w| w.iter().zip(folders.iter()).all(|(a, b)| a == b)))
        .then_some(*reason)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rekordbox::location::decode;

    fn path(raw: &str) -> FilePath {
        decode(raw).unwrap().as_file().unwrap().clone()
    }

    #[test]
    fn demo_tracks_and_samples_are_found_for_any_user_and_drive_ignoring_case() {
        for (raw, reason) in [
            (
                "file://localhost/C:/Users/someone/Music/PioneerDJ/Demo%20Tracks/Demo%20Track%201.mp3",
                SkipReason::DemoTrack,
            ),
            (
                "file://localhost/D:/Users/Other%20Person/music/PIONEERDJ/demo%20tracks/x.mp3",
                SkipReason::DemoTrack,
            ),
            (
                "file://localhost/C:/Users/someone/Music/rekordbox/Sampler/OSC_SAMPLER(3)/PRESET%20ONESHOT/NOISE.wav",
                SkipReason::Sample,
            ),
            (
                "file://localhost/C:/Users/someone/OneDrive/Music/rekordbox/Sampler/x.wav",
                SkipReason::Sample,
            ),
            (
                "file://localhost/Users/someone/Music/rekordbox/Sampler/x.wav",
                SkipReason::Sample,
            ),
            (
                "file://localhost//nas/home/Music/PioneerDJ/Demo%20Tracks/x.mp3",
                SkipReason::DemoTrack,
            ),
        ] {
            assert_eq!(built_in(&path(raw)), Some(reason), "{raw:?}");
        }
    }

    #[test]
    fn the_users_own_files_near_the_built_in_folders_are_not_skipped() {
        for raw in [
            "file://localhost/C:/Users/someone/Music/PioneerDJ/My%20Tracks/x.mp3",
            "file://localhost/C:/Users/someone/Music/rekordbox/Samplers/x.wav",
            "file://localhost/C:/Users/someone/Music/Demo%20Tracks/x.mp3",
            "file://localhost/C:/Users/someone/Music/rekordbox/Sampler.wav",
            "file://localhost/C:/Sampler/rekordbox/Music/x.wav",
            "file://localhost/C:/Music/PioneerDJ/Demo%20Tracks",
            // Not under a Music folder: the user's own.
            "file://localhost/D:/DJ/rekordbox/Sampler/x.wav",
            "file://localhost/E:/PioneerDJ/Demo%20Tracks/x.mp3",
        ] {
            assert_eq!(built_in(&path(raw)), None, "{raw:?}");
        }
    }
}
