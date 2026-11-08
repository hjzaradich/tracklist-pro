//! MusicBrainz recording search (ROADMAP §5.6). The limit is 1 request a
//! second and a User-Agent naming the app; the gate supplies both.

use serde_json::Value;

use super::{clamp_limit, get_ok, parse_json, text, urlencode, ClientError, LookupHit};
use crate::net::{Net, Request, Service};

const BASE: &str = "https://musicbrainz.org/ws/2";

/// Free-text search for a recording.
pub fn search(net: &Net, query: &str, limit: usize) -> Result<Vec<LookupHit>, ClientError> {
    search_at(net, BASE, query, limit)
}

/// [`search`] against `base`. Tests point it at a local server.
pub(super) fn search_at(
    net: &Net,
    base: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<LookupHit>, ClientError> {
    let url = format!(
        "{base}/recording?query={}&limit={}&fmt=json",
        urlencode(query),
        clamp_limit(limit)
    );
    let body = get_ok(net, || {
        Request::get(Service::MusicBrainz, url.clone()).header("Accept", "application/json")
    })?;
    parse_search(&body)
}

pub(super) fn parse_search(body: &[u8]) -> Result<Vec<LookupHit>, ClientError> {
    let value = parse_json(Service::MusicBrainz, body)?;
    let recordings = value
        .get("recordings")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(recordings.iter().map(hit).collect())
}

fn hit(recording: &Value) -> LookupHit {
    let first = |key: &str| {
        recording
            .get(key)
            .and_then(Value::as_array)
            .and_then(|list| list.first())
    };
    let release = first("releases");
    LookupHit {
        score: recording.get("score").and_then(Value::as_i64),
        title: text(recording, "title"),
        artist: first("artist-credit").and_then(|credit| text(credit, "name")),
        album: release.and_then(|r| text(r, "title")),
        year: release
            .and_then(|r| text(r, "date"))
            .and_then(|date| date.get(..4).and_then(|y| y.parse().ok())),
        isrc: first("isrcs").and_then(Value::as_str).map(str::to_owned),
        url: text(recording, "id").map(|id| format!("https://musicbrainz.org/recording/{id}")),
        release_id: release.and_then(|r| text(r, "id")),
        ..LookupHit::new(Service::MusicBrainz)
    }
}

/// A recording MusicBrainz knows (`GET /recording/<mbid>`): what the look-up
/// job needs to turn an AcoustID match into details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub id: String,
    pub title: Option<String>,
    /// The artist credit as written: each credited name followed by its join
    /// phrase ("A & B feat. C").
    pub artist: Option<String>,
    pub releases: Vec<ReleaseRef>,
}

/// A release a recording appears on, as the recording lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRef {
    pub id: String,
    pub title: Option<String>,
    /// "2001-05-06", "2001-05" or "2001".
    pub date: Option<String>,
    /// "Official", "Promotion", "Bootleg", ...
    pub status: Option<String>,
}

/// A release (`GET /release/<mbid>`) with its labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub id: String,
    pub title: Option<String>,
    pub date: Option<String>,
    pub labels: Vec<LabelInfo>,
}

/// One label of a release with the catalogue number it gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelInfo {
    pub name: Option<String>,
    pub catalogue_number: Option<String>,
}

/// The recording `mbid` with its artist credit and releases.
pub fn recording(net: &Net, mbid: &str) -> Result<Recording, ClientError> {
    recording_at(net, BASE, mbid)
}

/// [`recording`] against `base`. Tests point it at a local server.
pub(super) fn recording_at(net: &Net, base: &str, mbid: &str) -> Result<Recording, ClientError> {
    let url = entity_url(base, "recording", mbid, "artist-credits+releases")?;
    parse_recording(&fetch(net, url)?)
}

/// The release `mbid` with its labels, catalogue numbers and date.
pub fn release(net: &Net, mbid: &str) -> Result<Release, ClientError> {
    release_at(net, BASE, mbid)
}

/// [`release`] against `base`. Tests point it at a local server.
pub(super) fn release_at(net: &Net, base: &str, mbid: &str) -> Result<Release, ClientError> {
    let url = entity_url(base, "release", mbid, "labels+artist-credits")?;
    parse_release(&fetch(net, url)?)
}

/// `inc` is written with its own `+` separators, which are not encoded.
fn entity_url(base: &str, kind: &str, mbid: &str, inc: &str) -> Result<String, ClientError> {
    let mbid = mbid.trim();
    if mbid.is_empty() {
        return Err(ClientError::NoId);
    }
    Ok(format!(
        "{base}/{kind}/{}?inc={inc}&fmt=json",
        urlencode(mbid)
    ))
}

fn fetch(net: &Net, url: String) -> Result<Vec<u8>, ClientError> {
    get_ok(net, || {
        Request::get(Service::MusicBrainz, url.clone()).header("Accept", "application/json")
    })
}

pub(crate) fn parse_recording(body: &[u8]) -> Result<Recording, ClientError> {
    let value = parse_json(Service::MusicBrainz, body)?;
    let id = text(&value, "id").ok_or(ClientError::BadReply {
        service: Service::MusicBrainz,
        what: "the recording had no id",
    })?;
    Ok(Recording {
        id,
        title: text(&value, "title"),
        artist: artist_credit(&value),
        releases: list(&value, "releases")
            .iter()
            .filter_map(|r| {
                Some(ReleaseRef {
                    id: text(r, "id")?,
                    title: text(r, "title"),
                    date: text(r, "date"),
                    status: text(r, "status"),
                })
            })
            .collect(),
    })
}

pub(crate) fn parse_release(body: &[u8]) -> Result<Release, ClientError> {
    let value = parse_json(Service::MusicBrainz, body)?;
    let id = text(&value, "id").ok_or(ClientError::BadReply {
        service: Service::MusicBrainz,
        what: "the release had no id",
    })?;
    Ok(Release {
        id,
        title: text(&value, "title"),
        date: text(&value, "date"),
        labels: list(&value, "label-info")
            .iter()
            .map(|info| LabelInfo {
                name: info.get("label").and_then(|l| text(l, "name")),
                catalogue_number: text(info, "catalog-number"),
            })
            .filter(|l| l.name.is_some() || l.catalogue_number.is_some())
            .collect(),
    })
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The credit as MusicBrainz writes it out: each name (the credited one, else
/// the artist's own) followed by its join phrase, so "A", " & ", "B" reads
/// "A & B". `None` when no name is there.
fn artist_credit(value: &Value) -> Option<String> {
    let mut out = String::new();
    for credit in list(value, "artist-credit") {
        let Some(name) =
            text(credit, "name").or_else(|| credit.get("artist").and_then(|a| text(a, "name")))
        else {
            continue;
        };
        out.push_str(&name);
        // Not `text`: a join phrase is blanks around a word, and those matter.
        if let Some(join) = credit.get("joinphrase").and_then(Value::as_str) {
            out.push_str(join);
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!out.is_empty()).then_some(out)
}
