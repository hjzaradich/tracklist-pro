//! The online services the app can talk to, each with its own opt-in and
//! its own limits.

use std::time::Duration;

/// An online service. Every request names one, and the gate checks the
/// user's opt-in for it (`service_optin`) before anything is sent.
///
/// Model downloads and update checks count as services too (ROADMAP §1.1).
/// Phase 2 adds the remaining metadata services (AcoustID, Spotify) as
/// they're wired up, each with the limits its terms ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    /// MusicBrainz metadata lookups (ROADMAP §5.6).
    MusicBrainz,
    /// Checking for a newer version of the app (Phase 4).
    UpdateCheck,
    /// Downloading the on-device audio models (Phase 3).
    ModelDownload,
    /// Discogs release search (genre, label), with the user's own token.
    Discogs,
    /// Front covers from the Cover Art Archive. Its own switch: it talks to
    /// different hosts than MusicBrainz.
    CoverArt,
}

impl Service {
    /// Every service, for tests and the settings screen.
    pub const ALL: [Service; 5] = [
        Service::MusicBrainz,
        Service::UpdateCheck,
        Service::ModelDownload,
        Service::Discogs,
        Service::CoverArt,
    ];

    /// The service's key in `service_optin.service`: lowercase letters,
    /// digits and `_` (the table checks this).
    pub fn id(self) -> &'static str {
        match self {
            Service::MusicBrainz => "musicbrainz",
            Service::UpdateCheck => "update_check",
            Service::ModelDownload => "model_download",
            Service::Discogs => "discogs",
            Service::CoverArt => "cover_art",
        }
    }

    /// The hosts this service may talk to, lowercase. A request to any other
    /// host is refused before a connection is made (see [`Service::allows_host`]).
    ///
    /// An entry starting with `.` also covers every subdomain, e.g.
    /// `.archive.org` covers `ia800000.us.archive.org`. Model downloads
    /// have no hosts yet: until that feature is wired up and names its own
    /// here, every request to it is refused.
    pub fn hosts(self) -> &'static [&'static str] {
        match self {
            Service::MusicBrainz => &["musicbrainz.org"],
            // The Cover Art Archive hands images over from the Internet
            // Archive, which it redirects to.
            Service::CoverArt => &["coverartarchive.org", "archive.org", ".archive.org"],
            // The API only. The token is sent to this host and no other.
            Service::Discogs => &["api.discogs.com"],
            // The GitHub API only: the latest release of this project.
            Service::UpdateCheck => &["api.github.com"],
            Service::ModelDownload => &[],
        }
    }

    /// Whether `host` is one of this service's hosts. Whole-name match only:
    /// `evilmusicbrainz.org` and `musicbrainz.org.example.test` don't match
    /// `musicbrainz.org`.
    pub fn allows_host(self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.hosts()
            .iter()
            .any(|pinned| match pinned.strip_prefix('.') {
                Some(_) => host.ends_with(pinned),
                None => host == *pinned,
            })
    }

    /// The shortest time between two requests to this service. Requests
    /// wait their turn; none is ever sent early.
    pub fn min_interval(self) -> Duration {
        match self {
            // MusicBrainz allows 1 request per second (ROADMAP §5.6).
            Service::MusicBrainz => Duration::from_secs(1),
            // Discogs allows 60 requests a minute with a token; a little
            // over a second apart stays under that.
            Service::Discogs => Duration::from_millis(1100),
            // No published limit. Each cover can take a few requests (the
            // archive redirects twice), so this is shorter than the others.
            Service::CoverArt => Duration::from_millis(500),
            // No published limit. One a second is polite and plenty: these
            // are rare, user-started requests.
            Service::UpdateCheck | Service::ModelDownload => Duration::from_secs(1),
        }
    }

    /// The most a whole request may take, from connecting to the last byte
    /// of the response.
    pub fn timeout(self) -> Duration {
        match self {
            Service::MusicBrainz | Service::UpdateCheck | Service::Discogs | Service::CoverArt => {
                Duration::from_secs(30)
            }
            // Models are large and some connections are slow.
            Service::ModelDownload => Duration::from_secs(60 * 60),
        }
    }

    /// The largest response body accepted, in bytes. A bigger one is an
    /// error, not a truncated answer.
    pub fn max_body(self) -> u64 {
        const MIB: u64 = 1024 * 1024;
        match self {
            Service::MusicBrainz | Service::Discogs => 10 * MIB,
            // The most a cover may be; a bigger one isn't a cover.
            Service::CoverArt => 12 * MIB,
            // A release's JSON is a few KB; 256 KB is generous.
            Service::UpdateCheck => 256 * 1024,
            Service::ModelDownload => 1024 * MIB,
        }
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}
