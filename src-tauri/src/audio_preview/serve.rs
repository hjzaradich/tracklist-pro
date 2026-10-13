//! Answering one request: the URL, the Range header, the bytes and the
//! headers (1cA-7).

use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom};
use std::panic::{self, AssertUnwindSafe};

use tauri::http::{header, Method, Request, Response, StatusCode};

use super::open::{open, Opened};
use super::{PreviewRefusal, MAX_CHUNK};
use crate::db::ReadPool;
use crate::paths::Volumes;

/// The response header that names why a request was refused, for tests and
/// for anyone looking at the traffic. (A media element can't read it: the
/// player asks `preview_check` instead.)
pub(super) const REFUSAL_HEADER: &str = "x-preview-refusal";

pub(super) type Reply = Response<Cow<'static, [u8]>>;

/// The file id a request names: the path must be `/` and decimal digits,
/// nothing else. No other part of the URL is looked at, so no path, name or
/// `..` in it can reach a file.
pub(super) fn file_id(path: &str) -> Option<i64> {
    let digits = path.strip_prefix('/')?;
    if digits.is_empty() || digits.len() > 18 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// What a `Range` header asks for.
#[derive(Debug, PartialEq, Eq)]
enum Ask {
    /// No usable header: from the start.
    Start,
    /// `bytes=a-` or `bytes=a-b`.
    From(u64, Option<u64>),
    /// `bytes=-n`: the last n bytes.
    Last(u64),
}

fn parse_range(header: Option<&str>) -> Ask {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return Ask::Start;
    };
    // More than one range: the first is answered.
    let first = spec.split(',').next().unwrap_or("").trim();
    let Some((from, to)) = first.split_once('-') else {
        return Ask::Start;
    };
    let (from, to) = (from.trim(), to.trim());
    match (from.parse::<u64>(), to.parse::<u64>()) {
        (Ok(a), Ok(b)) => Ask::From(a, Some(b)),
        (Ok(a), Err(_)) if to.is_empty() => Ask::From(a, None),
        (Err(_), Ok(n)) if from.is_empty() => Ask::Last(n),
        _ => Ask::Start,
    }
}

/// The bytes to answer with, `start..=end` of a file of `total` bytes, or
/// `None` if the range starts past the end. A response never holds more
/// than [`MAX_CHUNK`] bytes: a longer ask is cut, and the media element
/// asks again for the rest.
fn resolve(ask: &Ask, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let (start, end) = match *ask {
        Ask::Start => (0, total - 1),
        Ask::From(a, b) => (a, b.map_or(total - 1, |b| b.min(total - 1))),
        Ask::Last(0) => return None,
        Ask::Last(n) => (total.saturating_sub(n), total - 1),
    };
    if start >= total || end < start {
        return None;
    }
    Some((start, end.min(start + MAX_CHUNK - 1)))
}

fn plain(status: StatusCode) -> http_response::Builder {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
}

use tauri::http::response as http_response;

pub(super) fn refused(refusal: PreviewRefusal) -> Reply {
    let status = match refusal {
        PreviewRefusal::UnknownFile | PreviewRefusal::Missing => StatusCode::NOT_FOUND,
        PreviewRefusal::DriveNotConnected => StatusCode::SERVICE_UNAVAILABLE,
        PreviewRefusal::OnlineOnly => StatusCode::FORBIDDEN,
        PreviewRefusal::NotDecodable => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        PreviewRefusal::Unreadable => StatusCode::INTERNAL_SERVER_ERROR,
    };
    plain(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .header(REFUSAL_HEADER, refusal.code())
        .body(Cow::Borrowed(&[][..]))
        .expect("a valid response")
}

fn status_only(status: StatusCode) -> Reply {
    plain(status)
        .body(Cow::Borrowed(&[][..]))
        .expect("a valid response")
}

/// Answers `request`: GET (or HEAD) of `/<file id>`, with an optional
/// `Range`. Runs on a worker thread; blocking is fine here.
pub(super) fn answer(
    reads: &ReadPool,
    volumes: &impl Volumes,
    request: &Request<Vec<u8>>,
) -> Reply {
    let head = match *request.method() {
        Method::GET => false,
        Method::HEAD => true,
        _ => return status_only(StatusCode::METHOD_NOT_ALLOWED),
    };
    let Some(id) = file_id(request.uri().path()) else {
        return status_only(StatusCode::BAD_REQUEST);
    };
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    let ask = parse_range(range);
    match contained(|| respond(reads, volumes, id, &ask, range.is_some(), head)) {
        Ok(reply) => reply,
        Err(refusal) => refused(refusal),
    }
}

/// Runs `work`, turning a panic into [`PreviewRefusal::NotDecodable`]. Decoding is
/// third-party code that sees whatever bytes a file holds: a panic in it is
/// that file failing to decode, never the app going down (as in the quality
/// job).
pub(super) fn contained<T>(
    work: impl FnOnce() -> Result<T, PreviewRefusal>,
) -> Result<T, PreviewRefusal> {
    panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(PreviewRefusal::NotDecodable))
}

fn respond(
    reads: &ReadPool,
    volumes: &impl Volumes,
    id: i64,
    ask: &Ask,
    ranged: bool,
    head: bool,
) -> Result<Reply, PreviewRefusal> {
    let mut opened = open(reads, volumes, id)?;
    let (total, mime) = match &opened {
        Opened::Native { len, mime, .. } => (*len, *mime),
        Opened::Decoded(d) => (d.layout.total(), "audio/wav"),
    };
    let Some((start, end)) = resolve(ask, total) else {
        return Ok(plain(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
            .body(Cow::Borrowed(&[][..]))
            .expect("a valid response"));
    };
    let length = end - start + 1;
    // A whole file that fits in one chunk is a plain 200, as a server that
    // ignored Range would answer; anything else is a partial answer.
    let partial = ranged || length < total;
    let body: Vec<u8> = if head {
        Vec::new()
    } else {
        match &mut opened {
            Opened::Native { file, .. } => {
                file.seek(SeekFrom::Start(start))
                    .map_err(|_| PreviewRefusal::Unreadable)?;
                let mut bytes = vec![0u8; length as usize];
                file.read_exact(&mut bytes)
                    .map_err(|_| PreviewRefusal::Unreadable)?;
                bytes
            }
            Opened::Decoded(decoded) => decoded.read(start, end)?,
        }
    };
    let mut reply = plain(if partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    })
    .header(header::CONTENT_TYPE, mime)
    .header(header::ACCEPT_RANGES, "bytes")
    .header(header::CONTENT_LENGTH, length);
    if partial {
        reply = reply.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        );
    }
    Ok(reply.body(Cow::Owned(body)).expect("a valid response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_digits_after_one_slash_name_a_file() {
        assert_eq!(file_id("/42"), Some(42));
        assert_eq!(file_id("/0"), Some(0));
        for bad in [
            "",
            "/",
            "42",
            "//42",
            "/42/",
            "/42/x",
            "/-1",
            "/+1",
            "/4 2",
            "/0x2a",
            "/..",
            "/../42",
            "/C:/music/a.mp3",
            "/%34%32",
            "/42?x=1",
            "/42.mp3",
            "/١٢",
            "/1234567890123456789",
        ] {
            assert_eq!(file_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn range_headers_are_understood_and_cut_to_a_chunk() {
        let total = 10 * MAX_CHUNK;
        let cases = [
            (None, (0, MAX_CHUNK - 1)),
            (Some("bytes=0-"), (0, MAX_CHUNK - 1)),
            (Some("bytes=5-9"), (5, 9)),
            (Some("bytes=5-"), (5, 5 + MAX_CHUNK - 1)),
            (Some("bytes=-100"), (total - 100, total - 1)),
            (Some("bytes=0-99999999999"), (0, MAX_CHUNK - 1)),
            (Some("bytes=3-4, 9-10"), (3, 4)),
            (Some("nonsense"), (0, MAX_CHUNK - 1)),
        ];
        for (header, expected) in cases {
            assert_eq!(
                resolve(&parse_range(header), total),
                Some(expected),
                "{header:?}"
            );
        }
    }

    #[test]
    fn the_last_partial_range_is_the_rest_of_the_file() {
        let total = 3 * MAX_CHUNK + 17;
        let start = 3 * MAX_CHUNK;
        assert_eq!(
            resolve(&parse_range(Some(&format!("bytes={start}-"))), total),
            Some((start, total - 1))
        );
        // Asking past the end is refused; asking for more than there is is
        // cut to what there is.
        assert_eq!(
            resolve(&parse_range(Some(&format!("bytes={total}-"))), total),
            None
        );
        assert_eq!(resolve(&parse_range(Some("bytes=-0")), total), None);
        assert_eq!(
            resolve(
                &parse_range(Some(&format!("bytes={start}-{}", total + 500))),
                total
            ),
            Some((start, total - 1))
        );
        assert_eq!(resolve(&Ask::Start, 0), None);
    }
}
