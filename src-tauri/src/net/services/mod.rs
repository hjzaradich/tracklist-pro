//! Clients for the online lookup services: MusicBrainz, Discogs and the
//! Cover Art Archive (ROADMAP §5.6). Each sends through the network gate
//! ([`Net::send`]), so each is off until the user opts in, waits its turn
//! behind the service's rate limit and may only reach the service's own
//! hosts.
//!
//! Nothing calls these yet: identification (ROADMAP 2.2) will. They are
//! copies of the old app's clients, reshaped to the gate.
//!
//! A client never sleeps. A retry is a new [`Net::send`], so the limiter
//! spaces it like any other request.

pub mod coverart;
pub mod discogs;
pub mod musicbrainz;
#[cfg(test)]
mod tests;

use serde_json::Value;

use super::{Net, NetError, Request, Service};

/// How many times a 503 ("busy, try again") is retried.
const MAX_RETRIES: u32 = 2;

/// The most results a search asks for. The services cap a page near here.
const MAX_LIMIT: usize = 100;

/// One result of a lookup, from one service. Every field but `source` may
/// be missing: services return different things.
#[derive(Debug, Clone, PartialEq)]
pub struct LookupHit {
    pub source: Service,
    /// The service's own confidence, 0 to 100, when it gives one.
    pub score: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i64>,
    pub isrc: Option<String>,
    /// Discogs supplies these; a MusicBrainz recording search doesn't.
    pub genre: Option<String>,
    pub label: Option<String>,
    /// A page about the result, to show the user. Never fetched.
    pub url: Option<String>,
    /// The release's MusicBrainz id, when the recording named one. Cover
    /// art hangs off the release, so a hit without one can't supply a cover.
    pub release_id: Option<String>,
}

impl LookupHit {
    fn new(source: Service) -> LookupHit {
        LookupHit {
            source,
            score: None,
            title: None,
            artist: None,
            album: None,
            year: None,
            isrc: None,
            genre: None,
            label: None,
            url: None,
            release_id: None,
        }
    }
}

/// Why a lookup failed. Says which service or host and what went wrong,
/// never the URL or the query, which can hold the user's search and token.
#[derive(Debug)]
pub enum ClientError {
    /// The gate refused or the request failed (not opted in, host not
    /// pinned, no connection, too large, ...).
    Net(NetError),
    /// The service has nothing at that address (404).
    NotFound(Service),
    /// The service refused the user's token (401 or 403).
    Unauthorized(Service),
    /// The service says to slow down (429).
    RateLimited(Service),
    /// Any other unsuccessful status. A 503 only gets here once the retries
    /// are used up.
    Status { service: Service, status: u16 },
    /// The service answered, but not with what was asked for.
    BadReply {
        service: Service,
        what: &'static str,
    },
    /// Discogs needs the user's token, and there isn't a usable one.
    NoToken,
    /// A lookup needs an id and was given none.
    NoId,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Net(e) => e.fmt(f),
            ClientError::NotFound(s) => write!(f, "{s} has nothing there"),
            ClientError::Unauthorized(s) => write!(f, "{s} refused the token"),
            ClientError::RateLimited(s) => write!(f, "{s} asked us to slow down"),
            ClientError::Status { service, status } => {
                write!(f, "{service} answered with status {status}")
            }
            ClientError::BadReply { service, what } => {
                write!(f, "{service} sent an unexpected reply: {what}")
            }
            ClientError::NoToken => f.write_str("a Discogs token is needed"),
            ClientError::NoId => f.write_str("an id is needed"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<NetError> for ClientError {
    fn from(e: NetError) -> ClientError {
        ClientError::Net(e)
    }
}

/// Sends the GET `make` builds and returns the body of a successful reply,
/// retrying a 503 up to [`MAX_RETRIES`] times. Every attempt goes through
/// [`Net::send`], so the service's rate limit spaces them.
fn get_ok(net: &Net, make: impl Fn() -> Request) -> Result<Vec<u8>, ClientError> {
    let mut attempt = 0;
    loop {
        let request = make();
        let service = request.service();
        let response = net.send(request)?;
        match response.status {
            200..=299 => return Ok(response.body),
            503 if attempt < MAX_RETRIES => attempt += 1,
            404 => return Err(ClientError::NotFound(service)),
            401 | 403 => return Err(ClientError::Unauthorized(service)),
            429 => return Err(ClientError::RateLimited(service)),
            status => return Err(ClientError::Status { service, status }),
        }
    }
}

/// A JSON reply's top-level value.
fn parse_json(service: Service, body: &[u8]) -> Result<Value, ClientError> {
    serde_json::from_slice(body).map_err(|_| ClientError::BadReply {
        service,
        what: "it wasn't JSON",
    })
}

/// A string field, trimmed; missing, not a string or empty gives `None`.
fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The search size to ask for: at least 1, at most [`MAX_LIMIT`].
fn clamp_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_LIMIT)
}

/// Minimal percent-encoding for query strings and path pieces. Anything but
/// the unreserved characters `A-Za-z0-9-_.~` is escaped, byte by byte.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
