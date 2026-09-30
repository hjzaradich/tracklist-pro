//! The online services the app can talk to, each with its own opt-in and
//! its own limits.

use std::time::Duration;

/// An online service. Every request names one, and the gate checks the
/// user's opt-in for it (`service_optin`) before anything is sent.
///
/// Model downloads and update checks count as services too (ROADMAP §1.1).
/// Phase 2 adds the metadata services (AcoustID,
/// Cover Art Archive, Discogs, Spotify) as they're wired up, each with the
/// limits its terms ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    /// MusicBrainz metadata lookups (ROADMAP §5.6).
    MusicBrainz,
    /// Checking for a newer version of the app (Phase 4).
    UpdateCheck,
    /// Downloading the on-device audio models (Phase 3).
    ModelDownload,
}

impl Service {
    /// Every service, for tests and the settings screen.
    pub const ALL: [Service; 3] = [
        Service::MusicBrainz,
        Service::UpdateCheck,
        Service::ModelDownload,
    ];

    /// The service's key in `service_optin.service`: lowercase letters,
    /// digits and `_` (the table checks this).
    pub fn id(self) -> &'static str {
        match self {
            Service::MusicBrainz => "musicbrainz",
            Service::UpdateCheck => "update_check",
            Service::ModelDownload => "model_download",
        }
    }

    /// The shortest time between two requests to this service. Requests
    /// wait their turn; none is ever sent early.
    pub fn min_interval(self) -> Duration {
        match self {
            // MusicBrainz allows 1 request per second (ROADMAP §5.6).
            Service::MusicBrainz => Duration::from_secs(1),
            // No published limit. One a second is polite and plenty: these
            // are rare, user-started requests.
            Service::UpdateCheck | Service::ModelDownload => Duration::from_secs(1),
        }
    }

    /// The most a whole request may take, from connecting to the last byte
    /// of the response.
    pub fn timeout(self) -> Duration {
        match self {
            Service::MusicBrainz | Service::UpdateCheck => Duration::from_secs(30),
            // Models are large and some connections are slow.
            Service::ModelDownload => Duration::from_secs(60 * 60),
        }
    }

    /// The largest response body accepted, in bytes. A bigger one is an
    /// error, not a truncated answer.
    pub fn max_body(self) -> u64 {
        const MIB: u64 = 1024 * 1024;
        match self {
            Service::MusicBrainz => 10 * MIB,
            Service::UpdateCheck => MIB,
            Service::ModelDownload => 1024 * MIB,
        }
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}
