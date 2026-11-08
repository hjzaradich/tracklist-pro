//! One file's online look-up (ROADMAP 2.2, owner decisions 2026-11-08; Run 6A,
//! S2a): the flow from a stored fingerprint to candidate details.
//!
//! 1. AcoustID, when it and MusicBrainz are switched on, the project's key is filled in and
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
//! sent for a service that is off or whose key is missing.
//!
//! **Name search (S2b).** When MusicBrainz is on and AcoustID found no
//! recording or was skipped (off, no key, no fingerprint), MusicBrainz is
//! searched by the file's current artist and title ([`super::names`]); a hit
//! counts only when it matches closely, and then the same recording and
//! release steps run. Its candidates say "MusicBrainz" with no service
//! leading to it. A file with no artist or title is skipped as `no_name`.
//! It is not used when AcoustID found a match.
//!
//! **Discogs (S2b).** When Discogs is on and the user's token is saved, its
//! release search by artist and title gives the label, year, catalogue number
//! and album of the closest hits (at most [`MAX_DISCOGS`]). No token: skipped
//! as `no_token`. Never an image. A Discogs failure keeps what the other
//! services already found.
//!
//! The services are reached through [`Backend`], so the flow is tested with
//! no network; the real one ([`Live`]) sends every request through the gate.

use super::names::{base_title, close, Names};
use super::offline::push_version;
use super::{Candidate, IdentifyField, IdentifySource};
use crate::credentials::{Credentials, DISCOGS};
use crate::net::services::acoustid::{self, AcoustidMatch};
use crate::net::services::discogs;
use crate::net::services::musicbrainz::{self, Recording, Release, ReleaseRef};
use crate::net::services::{ClientError, LookupHit};
use crate::net::{Net, NetError};
use crate::versions::parse_title;

/// The most hits a name search asks for.
const SEARCH_LIMIT: usize = 5;
/// The most Discogs hits whose details are kept.
pub const MAX_DISCOGS: usize = 3;

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
/// The name Discogs candidates give.
pub const DISCOGS_NAME: &str = "Discogs";

/// The requests a look-up makes.
pub trait Backend: Send + Sync + 'static {
    /// Whether the project's AcoustID key is still unfilled.
    fn acoustid_key_missing(&self) -> bool;
    fn acoustid(&self, blob: &[u8], seconds: u32) -> Result<Vec<AcoustidMatch>, ClientError>;
    fn recording(&self, mbid: &str) -> Result<Recording, ClientError>;
    fn release(&self, mbid: &str) -> Result<Release, ClientError>;
    /// MusicBrainz recording search with a query in its own syntax.
    fn search_recordings(&self, query: &str) -> Result<Vec<LookupHit>, ClientError>;
    /// Whether the user's Discogs token is saved.
    fn discogs_token_saved(&self) -> bool;
    /// Discogs release search by free text, with the user's token.
    fn discogs(&self, query: &str) -> Result<Vec<LookupHit>, ClientError>;
}

/// The real services, through the gate.
pub struct Live {
    pub net: Net,
    pub credentials: Credentials,
}

impl Backend for Live {
    fn acoustid_key_missing(&self) -> bool {
        acoustid::key_missing()
    }
    fn acoustid(&self, blob: &[u8], seconds: u32) -> Result<Vec<AcoustidMatch>, ClientError> {
        acoustid::lookup(&self.net, blob, seconds)
    }
    fn recording(&self, mbid: &str) -> Result<Recording, ClientError> {
        musicbrainz::recording(&self.net, mbid)
    }
    fn release(&self, mbid: &str) -> Result<Release, ClientError> {
        musicbrainz::release(&self.net, mbid)
    }
    fn search_recordings(&self, query: &str) -> Result<Vec<LookupHit>, ClientError> {
        musicbrainz::search(&self.net, query, SEARCH_LIMIT)
    }
    fn discogs_token_saved(&self) -> bool {
        matches!(self.credentials.get(DISCOGS), Ok(Some(t)) if !t.trim().is_empty())
    }
    fn discogs(&self, query: &str) -> Result<Vec<LookupHit>, ClientError> {
        // The token is read here, for this request, and goes nowhere else.
        let token = match self.credentials.get(DISCOGS) {
            Ok(Some(token)) => token,
            _ => return Err(ClientError::NoToken),
        };
        discogs::search(&self.net, &token, query, SEARCH_LIMIT * 2)
    }
}

/// Which services the user has switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct On {
    pub acoustid: bool,
    pub musicbrainz: bool,
    pub discogs: bool,
}

/// What a look-up needs from a file.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// `file.fingerprint`: the stored chromaprint fingerprint.
    pub fingerprint: Option<Vec<u8>>,
    pub duration_ms: Option<i64>,
    /// `file.fingerprint_rev` when the fingerprint was read: kept with the
    /// answer, so a later change of the audio shows the answer is out of date.
    pub fingerprint_rev: i64,
    /// The file's current artist and title, for the name search and Discogs.
    pub names: Option<Names>,
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
    /// The file has no artist or title to search by.
    NoName,
    /// Discogs is on but no token is saved.
    NoToken,
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
            Why::NoName => "no_name",
            Why::NoToken => "no_token",
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

/// The source of a candidate from AcoustID's recording: "MusicBrainz via
/// AcoustID".
pub fn source() -> IdentifySource {
    IdentifySource::Online {
        service: SERVICE.to_owned(),
        via: Some(VIA.to_owned()),
    }
}

/// The source of a candidate from the name search: "MusicBrainz".
pub fn name_source() -> IdentifySource {
    IdentifySource::Online {
        service: SERVICE.to_owned(),
        via: None,
    }
}

/// The source of a Discogs candidate.
pub fn discogs_source() -> IdentifySource {
    IdentifySource::Online {
        service: DISCOGS_NAME.to_owned(),
        via: None,
    }
}

/// How the services were asked about one file, so far.
#[derive(Default)]
struct Course {
    found: Vec<Candidate>,
    releases: Vec<String>,
    /// A service replied with nothing usable: why.
    nothing: Option<Why>,
    /// A service wasn't asked: why (the last one).
    skipped: Option<Why>,
}

/// Looks one file up. Sends nothing unless a service is on, its key is
/// there and the file has what the look-up needs. A request that fails ends
/// the look-up of this file as failed (a Discogs one only when nothing else
/// was found); it never panics or stops the caller.
pub fn look_up(backend: &dyn Backend, on: On, input: &Input) -> Answer {
    if !on.acoustid && !on.musicbrainz && !on.discogs {
        return Answer::skipped(Why::ServicesOff);
    }
    let mut course = Course::default();
    if let Err(answer) = musicbrainz_side(backend, on, input, &mut course) {
        return answer;
    }
    if on.discogs {
        if let Err(answer) = discogs_side(backend, input, &mut course) {
            if course.found.is_empty() {
                return answer;
            }
        }
    }
    if !course.found.is_empty() {
        return Answer {
            outcome: Outcome::Found,
            why: None,
            candidates: course.found,
            releases: course.releases,
        };
    }
    match course.nothing {
        Some(why) => Answer::bare(Outcome::Nothing, why),
        None => Answer::skipped(course.skipped.unwrap_or(Why::ServicesOff)),
    }
}

/// AcoustID, then MusicBrainz by its recordings, or by name when AcoustID
/// gave none.
fn musicbrainz_side(
    backend: &dyn Backend,
    on: On,
    input: &Input,
    course: &mut Course,
) -> Result<(), Answer> {
    let mut recordings: Vec<String> = Vec::new();
    if !on.acoustid {
        course.skipped = Some(Why::AcoustidOff);
    } else if !on.musicbrainz {
        // AcoustID only names recordings, which are of no use without
        // MusicBrainz: not asked.
        course.skipped = Some(Why::MusicbrainzOff);
    } else if backend.acoustid_key_missing() {
        course.skipped = Some(Why::KeyMissing);
    } else if let (Some(blob), Some(seconds)) = (
        input.fingerprint.as_deref(),
        acoustid::whole_seconds(input.duration_ms),
    ) {
        let matches = backend
            .acoustid(blob, seconds)
            .map_err(|e| error_answer(&e))?;
        recordings = best_recordings(matches);
        if recordings.is_empty() {
            course.nothing = Some(Why::NoMatch);
        }
    } else {
        course.skipped = Some(Why::NoFingerprint);
    }
    if !on.musicbrainz {
        return Ok(());
    }
    let mut origin = source();
    if recordings.is_empty() {
        origin = name_source();
        match &input.names {
            // AcoustID being off is the user's choice; a key or fingerprint that is
            // missing stays the reason given.
            None if matches!(course.skipped, None | Some(Why::AcoustidOff)) => {
                course.skipped = Some(Why::NoName);
            }
            None => {}
            Some(names) => match name_search(backend, names)? {
                Some(id) => recordings.push(id),
                None => {
                    course.nothing.get_or_insert(Why::NoMatch);
                }
            },
        }
    }
    if recordings.is_empty() {
        return Ok(());
    }
    details(backend, &recordings, &origin, course)?;
    if course.found.is_empty() {
        course.nothing.get_or_insert(Why::NoDetails);
    }
    Ok(())
}

/// The recordings' details and those of their releases.
fn details(
    backend: &dyn Backend,
    recordings: &[String],
    origin: &IdentifySource,
    course: &mut Course,
) -> Result<(), Answer> {
    let mut known = Vec::new();
    for mbid in recordings {
        match backend.recording(mbid) {
            Ok(recording) => {
                from_recording(&recording, origin, &mut course.found);
                known.push(recording);
            }
            // AcoustID named a recording MusicBrainz no longer has.
            Err(ClientError::NotFound(_) | ClientError::NoId) => {}
            Err(e) => return Err(error_answer(&e)),
        }
    }
    for reference in chosen_releases(&known) {
        match backend.release(&reference.id) {
            Ok(release) => {
                from_release(&release, origin, &mut course.found);
                course.releases.push(release.id);
            }
            Err(ClientError::NotFound(_) | ClientError::NoId) => {}
            Err(e) => return Err(error_answer(&e)),
        }
    }
    Ok(())
}

/// The id of the first recording the search finds that is the file's track.
fn name_search(backend: &dyn Backend, names: &Names) -> Result<Option<String>, Answer> {
    let query = format!(
        "recording:\"{}\" AND artist:\"{}\"",
        quoted(&base_title(&names.title)),
        quoted(&names.artist)
    );
    let hits = backend
        .search_recordings(&query)
        .map_err(|e| error_answer(&e))?;
    Ok(hits
        .into_iter()
        .filter(|h| matches_names(names, h))
        .find_map(|h| h.recording_id))
}

/// Text for the inside of a quoted search term.
fn quoted(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, '"' | '\\')).collect()
}

fn matches_names(names: &Names, hit: &LookupHit) -> bool {
    match (&hit.title, &hit.artist) {
        (Some(title), Some(artist)) => close(names, title, artist),
        _ => false,
    }
}

/// Discogs: the closest hits' label, year, catalogue number and album.
fn discogs_side(backend: &dyn Backend, input: &Input, course: &mut Course) -> Result<(), Answer> {
    if !backend.discogs_token_saved() {
        course.skipped = Some(Why::NoToken);
        return Ok(());
    }
    let Some(names) = &input.names else {
        course.skipped = Some(Why::NoName);
        return Ok(());
    };
    let query = format!("{} {}", names.artist, base_title(&names.title));
    let hits = backend.discogs(&query).map_err(|e| error_answer(&e))?;
    let close_hits: Vec<&LookupHit> = hits
        .iter()
        .filter(|h| matches_names(names, h))
        .take(MAX_DISCOGS)
        .collect();
    if close_hits.is_empty() {
        course.nothing.get_or_insert(Why::NoMatch);
    }
    let origin = discogs_source();
    for hit in close_hits {
        let mut add = |field, value: Option<String>| {
            if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                course.found.push(Candidate {
                    field,
                    value,
                    source: origin.clone(),
                });
            }
        };
        add(IdentifyField::Album, hit.title.clone());
        add(IdentifyField::Label, hit.label.clone());
        add(IdentifyField::CatalogueNumber, hit.catalogue_number.clone());
        add(
            IdentifyField::Year,
            hit.year
                .filter(|y| (1000..=9999).contains(y))
                .map(|y| y.to_string()),
        );
    }
    Ok(())
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

fn candidate(field: IdentifyField, value: String, source: &IdentifySource) -> Candidate {
    Candidate {
        field,
        value,
        source: source.clone(),
    }
}

/// Name, Artist and the version the title names.
fn from_recording(recording: &Recording, origin: &IdentifySource, out: &mut Vec<Candidate>) {
    if let Some(title) = &recording.title {
        // The title as MusicBrainz spells it, bracketed version included, as
        // the tags' title is offered; the version is also offered as the Mix.
        let mut versions = Vec::new();
        push_version(&parse_title(title), origin, &mut versions);
        out.push(candidate(IdentifyField::Name, title.clone(), origin));
        out.extend(versions);
    }
    if let Some(artist) = &recording.artist {
        out.push(candidate(IdentifyField::Artist, artist.clone(), origin));
    }
}

/// Album, Year, Label and catalogue number.
fn from_release(release: &Release, origin: &IdentifySource, out: &mut Vec<Candidate>) {
    if let Some(title) = &release.title {
        out.push(candidate(IdentifyField::Album, title.clone(), origin));
    }
    let year = release
        .date
        .as_deref()
        .and_then(|date| date.get(..4))
        .filter(|year| year.bytes().all(|b| b.is_ascii_digit()))
        // The years a send reads: 1000-9999 ("0000" and "0999" are not years).
        .filter(|year| year.parse::<u16>().is_ok_and(|y| y >= 1000));
    if let Some(year) = year {
        out.push(candidate(IdentifyField::Year, year.to_owned(), origin));
    }
    for label in &release.labels {
        if let Some(name) = &label.name {
            out.push(candidate(IdentifyField::Label, name.clone(), origin));
        }
        if let Some(number) = &label.catalogue_number {
            out.push(candidate(
                IdentifyField::CatalogueNumber,
                number.clone(),
                origin,
            ));
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
        ClientError::Net(e) if e.is_offline() => Answer::failed(Why::Offline),
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
