//! AcoustID fingerprint lookup (ROADMAP 2.2): a track's chromaprint
//! fingerprint in, MusicBrainz recording ids out.
//!
//! The request is a POST of an `application/x-www-form-urlencoded` body with
//! `client`, `meta=recordingids`, `duration` (whole seconds) and
//! `fingerprint`, to `api.acoustid.org` only. The gate checks the user's
//! opt-in, pins the host and spaces requests 3 a second.
//!
//! The application key is the project's own ([`ACOUSTID_CLIENT_KEY`]). While
//! it is empty the service is unavailable and nothing is sent.

use serde_json::Value;

use super::acoustid_key::ACOUSTID_CLIENT_KEY;
use super::{parse_json, urlencode, ClientError, MAX_RETRIES};
use crate::fingerprint::stored::{self, Fingerprint};
use crate::net::{Net, Request, Service};

const BASE: &str = "https://api.acoustid.org";

/// How much of a track is sent: the first two minutes, as AcoustID's own
/// tools do. The app stores the whole track's fingerprint.
const SENT_SECONDS: f32 = 120.0;

/// One result of a lookup: how well the fingerprint matched, and the
/// MusicBrainz recordings AcoustID knows for it.
#[derive(Debug, Clone, PartialEq)]
pub struct AcoustidMatch {
    /// 0 to 1.
    pub score: f64,
    /// MusicBrainz recording ids. Never empty: a result without any is
    /// dropped.
    pub recording_ids: Vec<String>,
}

/// Whether the project's key is still missing. Then [`lookup`] sends nothing.
pub fn key_missing() -> bool {
    key_is_missing(ACOUSTID_CLIENT_KEY)
}

fn key_is_missing(key: &str) -> bool {
    key.trim().is_empty()
}

/// Whole seconds for the lookup from a stored `duration_ms`, rounded to the
/// nearest; `None` for a missing, zero or negative length.
pub fn whole_seconds(duration_ms: Option<i64>) -> Option<u32> {
    let ms = duration_ms.filter(|&ms| ms > 0)?;
    u32::try_from((ms + 500) / 1000).ok().filter(|&s| s > 0)
}

/// Looks `blob`, a stored fingerprint (`file.fingerprint`), up. Best match
/// first. Sends nothing while the key is missing.
pub fn lookup(
    net: &Net,
    blob: &[u8],
    duration_secs: u32,
) -> Result<Vec<AcoustidMatch>, ClientError> {
    lookup_at(net, BASE, ACOUSTID_CLIENT_KEY, blob, duration_secs)
}

/// [`lookup`] against `base`, with `key`. Tests point it at a local server.
pub(crate) fn lookup_at(
    net: &Net,
    base: &str,
    key: &str,
    blob: &[u8],
    duration_secs: u32,
) -> Result<Vec<AcoustidMatch>, ClientError> {
    if key_is_missing(key) {
        return Err(ClientError::KeyMissing(Service::AcoustId));
    }
    if duration_secs == 0 {
        return Err(ClientError::NoFingerprint);
    }
    let fingerprint = encode(blob).ok_or(ClientError::NoFingerprint)?;
    let body = format!(
        "client={}&meta=recordingids&duration={duration_secs}&fingerprint={fingerprint}",
        urlencode(key.trim())
    );
    let url = format!("{base}/v2/lookup");
    let reply = post_form(net, &url, body.as_bytes())?;
    parse_lookup(&reply)
}

/// The fingerprint as AcoustID takes it: the stored bytes without the app's
/// own `TLFP` header and version byte, cut to the first two minutes,
/// compressed the way chromaprint does, then URL-safe base64 with no
/// padding. `None` for bytes that aren't a usable stored fingerprint.
pub(crate) fn encode(blob: &[u8]) -> Option<String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let fingerprint = Fingerprint::from_blob(blob).ok()?;
    let config = stored::config();
    let wanted = (SENT_SECONDS / config.item_duration_in_seconds()).ceil() as usize;
    let items = fingerprint.items();
    let items = &items[..items.len().min(wanted)];
    if items.is_empty() {
        return None;
    }
    let compressed = rusty_chromaprint::FingerprintCompressor::from(&config).compress(items);
    Some(URL_SAFE_NO_PAD.encode(compressed))
}

/// POSTs the form and returns the body of an answer to read: a success, or
/// the 400 AcoustID uses for its own errors (which come with a JSON reason).
/// A 503 is retried like the other clients' are.
fn post_form(net: &Net, url: &str, body: &[u8]) -> Result<Vec<u8>, ClientError> {
    let mut attempt = 0;
    loop {
        let request = Request::post(Service::AcoustId, url, body.to_vec())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json");
        let response = net.send(request)?;
        match response.status {
            200..=299 | 400 => return Ok(response.body),
            503 if attempt < MAX_RETRIES => attempt += 1,
            401 | 403 => return Err(ClientError::Unauthorized(Service::AcoustId)),
            429 => return Err(ClientError::RateLimited(Service::AcoustId)),
            status => {
                return Err(ClientError::Status {
                    service: Service::AcoustId,
                    status,
                })
            }
        }
    }
}

pub(super) fn parse_lookup(body: &[u8]) -> Result<Vec<AcoustidMatch>, ClientError> {
    let service = Service::AcoustId;
    let value = parse_json(service, body)?;
    match value.get("status").and_then(Value::as_str) {
        Some("ok") => {}
        Some("error") => return Err(service_error(&value)),
        _ => {
            return Err(ClientError::BadReply {
                service,
                what: "it had no status",
            })
        }
    }
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut matches: Vec<AcoustidMatch> = results.iter().filter_map(result).collect();
    // Best first; equal scores keep AcoustID's order.
    matches.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(matches)
}

fn result(result: &Value) -> Option<AcoustidMatch> {
    let score = result.get("score").and_then(Value::as_f64)?;
    let recording_ids: Vec<String> = result
        .get("recordings")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|r| r.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    if recording_ids.is_empty() || !score.is_finite() {
        return None;
    }
    Some(AcoustidMatch {
        score: score.clamp(0.0, 1.0),
        recording_ids,
    })
}

/// AcoustID's error codes that mean something to the caller; the rest are
/// kept as the code alone. Its message text is never kept.
fn service_error(value: &Value) -> ClientError {
    let service = Service::AcoustId;
    let code = value
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(Value::as_i64);
    match code {
        // Invalid application key, invalid user key.
        Some(4 | 6) => ClientError::Unauthorized(service),
        // Too many requests.
        Some(14) => ClientError::RateLimited(service),
        Some(code) => ClientError::ServiceError { service, code },
        None => ClientError::BadReply {
            service,
            what: "its error had no code",
        },
    }
}

#[cfg(test)]
mod tests;
