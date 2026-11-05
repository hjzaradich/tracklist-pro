//! Discogs release search, for the genre and label MusicBrainz doesn't
//! give. Needs the user's own token; the caller passes it in (storing it
//! comes with identification, ROADMAP 2.2). The token goes out as a header
//! on requests built here, and the gate pins those to Discogs' own hosts
//! ([`Service::hosts`]), so it can't reach another host.
//!
//! TODO(2.2): before this is shown to users, check Discogs' API terms for
//! attribution and for what may be done with the images, and meet them.

use serde_json::Value;

use super::{clamp_limit, get_ok, parse_json, text, urlencode, ClientError, LookupHit};
use crate::net::{Net, Request, Service};

const BASE: &str = "https://api.discogs.com";

/// Release search by free text.
pub fn search(
    net: &Net,
    token: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<LookupHit>, ClientError> {
    search_at(net, BASE, token, query, limit)
}

/// Whether Discogs accepts `token`: it can say who the token belongs to.
/// Doesn't return the account name.
pub fn check_token(net: &Net, token: &str) -> Result<(), ClientError> {
    check_token_at(net, BASE, token)
}

/// [`search`] against `base`. Tests point it at a local server.
pub(super) fn search_at(
    net: &Net,
    base: &str,
    token: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<LookupHit>, ClientError> {
    let header = auth_header(token)?;
    let url = format!(
        "{base}/database/search?q={}&type=release&per_page={}",
        urlencode(query),
        clamp_limit(limit)
    );
    let body = get_ok(net, || {
        Request::get(Service::Discogs, url.clone())
            .header("Authorization", header.clone())
            .header("Accept", "application/json")
    })?;
    parse_search(&body)
}

pub(super) fn check_token_at(net: &Net, base: &str, token: &str) -> Result<(), ClientError> {
    let header = auth_header(token)?;
    let url = format!("{base}/oauth/identity");
    let body = get_ok(net, || {
        Request::get(Service::Discogs, url.clone())
            .header("Authorization", header.clone())
            .header("Accept", "application/json")
    })?;
    match parse_json(Service::Discogs, &body)?.get("username") {
        Some(Value::String(name)) if !name.is_empty() => Ok(()),
        _ => Err(ClientError::BadReply {
            service: Service::Discogs,
            what: "it named no account",
        }),
    }
}

/// `Authorization: Discogs token=…`, or [`ClientError::NoToken`] for an
/// empty token or one that couldn't be a header value. Nothing is sent
/// then.
fn auth_header(token: &str) -> Result<String, ClientError> {
    let token = token.trim();
    if token.is_empty() || token.chars().any(char::is_control) {
        return Err(ClientError::NoToken);
    }
    Ok(format!("Discogs token={token}"))
}

pub(super) fn parse_search(body: &[u8]) -> Result<Vec<LookupHit>, ClientError> {
    let value = parse_json(Service::Discogs, body)?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(results.iter().map(hit).collect())
}

fn hit(result: &Value) -> LookupHit {
    // Discogs returns "Artist - Title" in one string.
    let combined = text(result, "title");
    let (artist, title) = match combined.as_deref().and_then(|c| c.split_once(" - ")) {
        Some((artist, title)) => (
            Some(artist.trim().to_owned()),
            Some(title.trim().to_owned()),
        ),
        None => (None, combined),
    };
    LookupHit {
        title,
        artist,
        // `style` is the more specific of the two ("Deep House" rather than
        // "Electronic"), so it wins where present.
        genre: first_string(result, "style").or_else(|| first_string(result, "genre")),
        label: first_string(result, "label"),
        year: result.get("year").and_then(|v| match v {
            Value::String(s) => s.trim().parse().ok(),
            other => other.as_i64(),
        }),
        url: text(result, "uri").map(|uri| format!("https://www.discogs.com{uri}")),
        ..LookupHit::new(Service::Discogs)
    }
}

/// First entry of a JSON string array, trimmed and non-empty.
fn first_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
