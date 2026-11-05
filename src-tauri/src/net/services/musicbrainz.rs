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
