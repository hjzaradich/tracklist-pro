//! Front covers from the Cover Art Archive. Artwork hangs off a MusicBrainz
//! *release* id. The archive answers with a redirect to the Internet
//! Archive; those hops are followed here, by hand, each one a new request
//! through the gate (so each is opt-in checked, rate limited and pinned to
//! [`Service::CoverArt`]'s hosts).

use super::{urlencode, ClientError};
use crate::net::{Net, Request, Service};

const BASE: &str = "https://coverartarchive.org";

/// The most redirects followed for one cover.
const MAX_HOPS: u32 = 3;

/// A cover image, as the archive sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artwork {
    pub bytes: Vec<u8>,
    /// The media type, always `image/...`.
    pub mime: String,
}

/// The front cover for a release. `Ok(None)` means the release has no cover
/// (404 or an empty body): a normal outcome, not a failure. A reply that
/// isn't an image, or is bigger than [`Service::max_body`] allows, is an
/// error.
pub fn front(net: &Net, release_id: &str) -> Result<Option<Artwork>, ClientError> {
    front_at(net, BASE, release_id)
}

/// [`front`] against `base`. Tests point it at a local server.
pub(super) fn front_at(
    net: &Net,
    base: &str,
    release_id: &str,
) -> Result<Option<Artwork>, ClientError> {
    let release_id = release_id.trim();
    if release_id.is_empty() {
        return Err(ClientError::NoId);
    }
    // The full-size image: `?maxwidth=` 404s for art that hasn't been
    // thumbnailed, while the original exists whenever any art does.
    let mut url = format!("{base}/release/{}/front", urlencode(release_id));
    for _ in 0..=MAX_HOPS {
        let response = net.send(Request::get(Service::CoverArt, url.clone()))?;
        match response.status {
            200..=299 => return artwork(response.content_type, response.body),
            301 | 302 | 303 | 307 | 308 => {
                let location = response.location.ok_or(ClientError::BadReply {
                    service: Service::CoverArt,
                    what: "a redirect with no place to go",
                })?;
                url = resolve(&url, location);
            }
            404 => return Ok(None),
            status => {
                return Err(ClientError::Status {
                    service: Service::CoverArt,
                    status,
                })
            }
        }
    }
    Err(ClientError::BadReply {
        service: Service::CoverArt,
        what: "too many redirects",
    })
}

/// Where a redirect from `current` points. A path alone (`/x`) stays on the
/// same host; anything else is taken as it is, and the gate refuses it
/// unless it names one of the service's hosts.
fn resolve(current: &str, location: String) -> String {
    if location.starts_with('/') && !location.starts_with("//") {
        if let Some(start) = current.find("://").map(|i| i + 3) {
            let end = current[start..]
                .find('/')
                .map_or(current.len(), |i| start + i);
            return format!("{}{location}", &current[..end]);
        }
    }
    location
}

fn artwork(content_type: Option<String>, bytes: Vec<u8>) -> Result<Option<Artwork>, ClientError> {
    let mime = content_type.unwrap_or_default();
    if !mime.starts_with("image/") {
        return Err(ClientError::BadReply {
            service: Service::CoverArt,
            what: "it wasn't an image",
        });
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(Artwork { bytes, mime }))
}
