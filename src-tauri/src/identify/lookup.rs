//! One file's online look-up (ROADMAP 2.2, owner decisions 2026-11-08; Run 6A,
//! S2a): the flow from a stored fingerprint to candidate details.
//!
//! 1. AcoustID, when it is switched on, the project's key is filled in and
//!    the file has a stored chromaprint fingerprint (`file.fingerprint`;
//!    never the audio model's `embedding`) and a length: the matches with a
//!    score of at least [`MIN_SCORE`], best first, give at most
//!    [`MAX_RECORDINGS`] recording ids.
//! 2. MusicBrainz, when it is switched on: each recording's title and artist
//!    credit, then at most [`MAX_RELEASES`] of the releases they appear on
//!    (official first, then the earliest), with label, catalogue number,
//!    date and title.
//!
//! The answer ([`Answer`]) says what came of it in one of four ways: details
//! found, nothing found, failed (and why) or skipped (and why). Nothing is
//! sent for a service that is off or whose key is missing. The text search
//! and Discogs are not here (S2b).
//!
//! The services are reached through [`Backend`], so the flow is tested with
//! no network; the real one ([`Live`]) sends every request through the gate.

use super::offline::push_version;
use super::{Candidate, IdentifyField, IdentifySource};
use crate::net::services::acoustid::{self, AcoustidMatch};
use crate::net::services::musicbrainz::{self, Recording, Release, ReleaseRef};
use crate::net::services::ClientError;
use crate::net::{Net, NetError};
use crate::versions::parse_title;

/// The lowest AcoustID score that counts as the same recording.
pub const MIN_SCORE: f64 = 0.8;
/// The most recordings asked about for a file.
pub const MAX_RECORDINGS: usize = 3;
/// The most releases asked about for a file.
pub const MAX_RELEASES: usize = 2;
/// The services asked, in order, as stored with the answer.
pub const PATH: &str = "acoustid+musicbrainz";

/// The name candidates give for their service and the one that led to it.
pub const SERVICE: &str = "MusicBrainz";
pub const VIA: &str = "AcoustID";

/// The requests a look-up makes.
pub trait Backend: Send + Sync + 'static {
    /// Whether the project's AcoustID key is still unfilled.
    fn acoustid_key_missing(&self) -> bool;
    fn acoustid(&self, blob: &[u8], seconds: u32) -> Result<Vec<AcoustidMatch>, ClientError>;
    fn recording(&self, mbid: &str) -> Result<Recording, ClientError>;
    fn release(&self, mbid: &str) -> Result<Release, ClientError>;
}

/// The real services, through the gate.
pub struct Live(pub Net);

impl Backend for Live {
    fn acoustid_key_missing(&self) -> bool {
        acoustid::key_missing()
    }
    fn acoustid(&self, blob: &[u8], seconds: u32) -> Result<Vec<AcoustidMatch>, ClientError> {
        acoustid::lookup(&self.0, blob, seconds)
    }
    fn recording(&self, mbid: &str) -> Result<Recording, ClientError> {
        musicbrainz::recording(&self.0, mbid)
    }
    fn release(&self, mbid: &str) -> Result<Release, ClientError> {
        musicbrainz::release(&self.0, mbid)
    }
}

/// Which services the user has switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct On {
    pub acoustid: bool,
    pub musicbrainz: bool,
}

/// What a look-up needs from a file.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// `file.fingerprint`: the stored chromaprint fingerprint.
    pub fingerprint: Option<Vec<u8>>,
    pub duration_ms: Option<i64>,
}

/// How a look-up came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Found,
    Nothing,
    Failed,
    Skipped,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Found => "found",
            Outcome::Nothing => "nothing",
            Outcome::Failed => "failed",
            Outcome::Skipped => "skipped",
        }
    }

    pub fn parse(name: &str) -> Option<Outcome> {
        [
            Outcome::Found,
            Outcome::Nothing,
            Outcome::Failed,
            Outcome::Skipped,
        ]
        .into_iter()
        .find(|o| o.as_str() == name)
    }

    /// An answer worth keeping over an older one: the services were asked and
    /// replied.
    pub fn completed(self) -> bool {
        matches!(self, Outcome::Found | Outcome::Nothing)
    }
}

/// Why a look-up found nothing, failed or was skipped: a short code the
/// screen words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Every online service is off.
    ServicesOff,
    AcoustidOff,
    MusicbrainzOff,
    KeyMissing,
    NoFingerprint,
    /// AcoustID knew no recording for it, or none it was sure of.
    NoMatch,
    /// MusicBrainz had nothing usable for the recordings.
    NoDetails,
    /// The internet couldn't be reached.
    Offline,
    /// A service asked us to slow down or was busy.
    Busy,
    /// A service refused the request.
    Refused,
    /// A service answered with something that wasn't what was asked for.
    BadReply,
    Other,
}

impl Why {
    pub fn code(self) -> &'static str {
        match self {
            Why::ServicesOff => "services_off",
            Why::AcoustidOff => "acoustid_off",
            Why::MusicbrainzOff => "musicbrainz_off",
            Why::KeyMissing => "key_missing",
            Why::NoFingerprint => "no_fingerprint",
            Why::NoMatch => "no_match",
            Why::NoDetails => "no_details",
            Why::Offline => "offline",
            Why::Busy => "busy",
            Why::Refused => "refused",
            Why::BadReply => "bad_reply",
            Why::Other => "failed",
        }
    }
}

/// What one file's look-up came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub outcome: Outcome,
    pub why: Option<Why>,
    pub candidates: Vec<Candidate>,
    /// The MusicBrainz release ids the candidates came from.
    pub releases: Vec<String>,
}

impl Answer {
    fn bare(outcome: Outcome, why: Why) -> Answer {
        Answer {
            outcome,
            why: Some(why),
            candidates: Vec::new(),
            releases: Vec::new(),
        }
    }

    pub fn skipped(why: Why) -> Answer {
        Answer::bare(Outcome::Skipped, why)
    }

    pub fn failed(why: Why) -> Answer {
        Answer::bare(Outcome::Failed, why)
    }
}

/// The source every candidate from this flow names: "MusicBrainz via
/// AcoustID".
pub fn source() -> IdentifySource {
    IdentifySource::Online {
        service: SERVICE.to_owned(),
        via: Some(VIA.to_owned()),
    }
}

/// Looks one file up. Sends nothing unless a service is on, its key is
/// there and the file has what the look-up needs. A request that fails ends
/// the look-up of this file as failed; it never panics or stops the caller.
pub fn look_up(backend: &dyn Backend, on: On, input: &Input) -> Answer {
    if !on.acoustid && !on.musicbrainz {
        return Answer::skipped(Why::ServicesOff);
    }
    if !on.acoustid {
        return Answer::skipped(Why::AcoustidOff);
    }
    if backend.acoustid_key_missing() {
        return Answer::skipped(Why::KeyMissing);
    }
    let (Some(blob), Some(seconds)) = (
        input.fingerprint.as_deref(),
        acoustid::whole_seconds(input.duration_ms),
    ) else {
        return Answer::skipped(Why::NoFingerprint);
    };
    let matches = match backend.acoustid(blob, seconds) {
        Ok(matches) => matches,
        Err(e) => return error_answer(&e),
    };
    let recordings = best_recordings(matches);
    if recordings.is_empty() {
        return Answer::bare(Outcome::Nothing, Why::NoMatch);
    }
    if !on.musicbrainz {
        return Answer::skipped(Why::MusicbrainzOff);
    }
    let mut found = Vec::new();
    let mut known = Vec::new();
    for mbid in &recordings {
        match backend.recording(mbid) {
            Ok(recording) => {
                from_recording(&recording, &mut found);
                known.push(recording);
            }
            // AcoustID named a recording MusicBrainz no longer has.
            Err(ClientError::NotFound(_)) => {}
            Err(e) => return error_answer(&e),
        }
    }
    let mut releases = Vec::new();
    for reference in chosen_releases(&known) {
        match backend.release(&reference.id) {
            Ok(release) => {
                from_release(&release, &mut found);
                releases.push(release.id);
            }
            Err(ClientError::NotFound(_)) => {}
            Err(e) => return error_answer(&e),
        }
    }
    if found.is_empty() {
        return Answer::bare(Outcome::Nothing, Why::NoDetails);
    }
    Answer {
        outcome: Outcome::Found,
        why: None,
        candidates: found,
        releases,
    }
}

/// The recording ids of the matches AcoustID is sure of, best match first,
/// each once, at most [`MAX_RECORDINGS`].
fn best_recordings(mut matches: Vec<AcoustidMatch>) -> Vec<String> {
    matches.retain(|m| m.score >= MIN_SCORE);
    matches.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut out: Vec<String> = Vec::new();
    for id in matches.into_iter().flat_map(|m| m.recording_ids) {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out.truncate(MAX_RECORDINGS);
    out
}

/// The releases to ask about: official ones first, then the earliest date
/// (one without a date last), each release once, at most [`MAX_RELEASES`].
fn chosen_releases(recordings: &[Recording]) -> Vec<ReleaseRef> {
    let mut all: Vec<ReleaseRef> = Vec::new();
    for release in recordings.iter().flat_map(|r| &r.releases) {
        if !all.iter().any(|seen| seen.id == release.id) {
            all.push(release.clone());
        }
    }
    all.sort_by_key(|r| {
        (
            r.status.as_deref() != Some("Official"),
            r.date.is_none(),
            r.date.clone(),
        )
    });
    all.truncate(MAX_RELEASES);
    all
}

fn candidate(field: IdentifyField, value: String) -> Candidate {
    Candidate {
        field,
        value,
        source: source(),
    }
}

/// Name, Artist and the version the title names.
fn from_recording(recording: &Recording, out: &mut Vec<Candidate>) {
    if let Some(title) = &recording.title {
        // The title as MusicBrainz spells it, bracketed version included, as
        // the tags' title is offered; the version is also offered as the Mix.
        let mut versions = Vec::new();
        push_version(&parse_title(title), &source(), &mut versions);
        out.push(candidate(IdentifyField::Name, title.clone()));
        out.extend(versions);
    }
    if let Some(artist) = &recording.artist {
        out.push(candidate(IdentifyField::Artist, artist.clone()));
    }
}

/// Album, Year, Label and catalogue number.
fn from_release(release: &Release, out: &mut Vec<Candidate>) {
    if let Some(title) = &release.title {
        out.push(candidate(IdentifyField::Album, title.clone()));
    }
    let year = release
        .date
        .as_deref()
        .and_then(|date| date.get(..4))
        .filter(|year| year.bytes().all(|b| b.is_ascii_digit()));
    if let Some(year) = year {
        out.push(candidate(IdentifyField::Year, year.to_owned()));
    }
    for label in &release.labels {
        if let Some(name) = &label.name {
            out.push(candidate(IdentifyField::Label, name.clone()));
        }
        if let Some(number) = &label.catalogue_number {
            out.push(candidate(IdentifyField::CatalogueNumber, number.clone()));
        }
    }
}

/// What a client error means for the file.
fn error_answer(error: &ClientError) -> Answer {
    match error {
        ClientError::KeyMissing(_) => Answer::skipped(Why::KeyMissing),
        ClientError::NoFingerprint => Answer::skipped(Why::NoFingerprint),
        // Switched off while the job ran.
        ClientError::Net(NetError::NotOptedIn(_)) => Answer::skipped(Why::ServicesOff),
        ClientError::Net(NetError::Failed { reason, .. }) if is_offline(reason) => {
            Answer::failed(Why::Offline)
        }
        ClientError::RateLimited(_) | ClientError::Status { status: 503, .. } => {
            Answer::failed(Why::Busy)
        }
        ClientError::Unauthorized(_) | ClientError::ServiceError { .. } => {
            Answer::failed(Why::Refused)
        }
        ClientError::BadReply { .. } | ClientError::NotFound(_) => Answer::failed(Why::BadReply),
        _ => Answer::failed(Why::Other),
    }
}

/// The gate's reasons for "no way to the internet" (`net::failure_reason`):
/// the host can't be found, the connection fails or times out.
fn is_offline(reason: &str) -> bool {
    matches!(
        reason,
        "the host wasn't found" | "the connection failed" | "it timed out"
    )
}
