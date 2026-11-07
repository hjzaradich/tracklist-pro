//! The update check (ROADMAP 4.5): asks GitHub for the project's latest
//! release and reports its version and page address. It only reads; nothing
//! is downloaded or run. The request goes through the gate, so it happens
//! only when the user has turned update checks on, and only to
//! `api.github.com`.
//!
//! Anything unexpected (no release yet, a pre-release, a draft, a reply
//! that isn't a release) is "no usable release": never an error the user
//! has to deal with.

use serde_json::Value;

use super::ClientError;
use crate::net::{Net, Request, Service};

const BASE: &str = "https://api.github.com";

/// The one path asked for.
const LATEST: &str = "/repos/hjzaradich/tracklist-pro/releases/latest";

/// Where release pages live. A reply naming any other address is ignored,
/// since the address is shown to the user to copy.
const RELEASE_PAGES: &str = "https://github.com/hjzaradich/tracklist-pro/releases/";

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `major.minor.patch`, without a leading `v`.
    pub version: String,
    /// The release's page, for the user to copy.
    pub url: String,
}

/// The latest published release, or `None` when there is no usable one.
pub fn latest(net: &Net) -> Result<Option<Release>, ClientError> {
    latest_at(net, BASE)
}

/// [`latest`] against `base`. Tests point it at a local server.
pub(super) fn latest_at(net: &Net, base: &str) -> Result<Option<Release>, ClientError> {
    let request = Request::get(Service::UpdateCheck, format!("{base}{LATEST}"))
        .header("Accept", "application/vnd.github+json");
    let response = net.send(request)?;
    match response.status {
        200..=299 => Ok(parse_release(&response.body)),
        // No release has been published yet.
        404 => Ok(None),
        status => Err(ClientError::Status {
            service: Service::UpdateCheck,
            status,
        }),
    }
}

/// A release from GitHub's reply; `None` for anything that isn't a plain
/// published release with a `major.minor.patch` tag and a page on the
/// project's releases.
pub(super) fn parse_release(body: &[u8]) -> Option<Release> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let flagged = |key: &str| value.get(key).and_then(Value::as_bool) == Some(true);
    if flagged("prerelease") || flagged("draft") {
        return None;
    }
    let version = parse_version(value.get("tag_name")?.as_str()?)?;
    let url = value.get("html_url")?.as_str()?.trim();
    if !url.starts_with(RELEASE_PAGES) || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(Release {
        version: format!("{}.{}.{}", version.0, version.1, version.2),
        url: url.to_owned(),
    })
}

/// `1.2.3` or `v1.2.3`. A pre-release (`1.2.3-beta`), build metadata or
/// anything else gives `None`.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let mut number = || {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<u64>().ok()
    };
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

/// Whether `release` is newer than `current` (the app's own version).
/// Unreadable on either side means no.
pub fn is_newer(release: &str, current: &str) -> bool {
    match (parse_version(release), parse_version(current)) {
        (Some(release), Some(current)) => release > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::tests::{test_db, Reply, TestServer};
    use crate::net::{set_opt_in, NetError};

    fn release_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"{RELEASE_PAGES}tag/{tag}","draft":false,"prerelease":false,"name":"x"}}"#
        )
    }

    fn json(body: &str) -> Reply {
        Reply::Typed(200, "application/json", body.as_bytes().to_vec())
    }

    #[test]
    fn a_tag_with_or_without_v_is_read_as_a_version() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("10.0.12"), Some((10, 0, 12)));
    }

    #[test]
    fn pre_releases_and_odd_tags_are_not_versions() {
        for tag in [
            "v0.2.0-beta.1",
            "0.2",
            "0.2.0.1",
            "v",
            "",
            "a.b.c",
            "0.2.0+x",
            "-1.0.0",
        ] {
            assert_eq!(parse_version(tag), None, "{tag:?}");
        }
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.9", "0.1.9"));
        assert!(!is_newer("0.1.8", "0.1.9"));
        assert!(!is_newer("junk", "0.1.9"));
        assert!(!is_newer("0.2.0", "junk"));
    }

    #[test]
    fn a_published_release_is_read() {
        let release = parse_release(release_json("v0.2.0").as_bytes()).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(
            release.url,
            "https://github.com/hjzaradich/tracklist-pro/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn a_reply_that_is_not_a_usable_release_gives_nothing() {
        let page = |url: &str| format!(r#"{{"tag_name":"v0.2.0","html_url":"{url}"}}"#);
        let cases = [
            "not json".to_owned(),
            "[]".to_owned(),
            "{}".to_owned(),
            r#"{"tag_name":"v0.2.0"}"#.to_owned(),
            format!(r#"{{"html_url":"{RELEASE_PAGES}tag/v0.2.0"}}"#),
            format!(r#"{{"tag_name":7,"html_url":"{RELEASE_PAGES}tag/x"}}"#),
            release_json("v0.2.0-rc.1"),
            release_json("v0.2.0").replace(r#""prerelease":false"#, r#""prerelease":true"#),
            release_json("v0.2.0").replace(r#""draft":false"#, r#""draft":true"#),
            page("https://evil.example/releases/tag/v0.2.0"),
            page(&format!("{RELEASE_PAGES}tag/v 0.2.0")),
            page("javascript:alert(1)"),
        ];
        for body in cases {
            assert_eq!(parse_release(body.as_bytes()), None, "{body}");
        }
    }

    #[test]
    fn the_check_asks_for_the_latest_release_and_nothing_else() {
        let db = test_db();
        set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
        let net = Net::for_tests(db.reads.clone());
        let server = TestServer::start(json(&release_json("v0.2.0")));

        let release = latest_at(&net, &server.url("")).unwrap().unwrap();

        assert_eq!(release.version, "0.2.0");
        let hits = server.hits();
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .request_line
            .starts_with("GET /repos/hjzaradich/tracklist-pro/releases/latest HTTP/1.1"));
        assert_eq!(
            hits[0].header("accept"),
            Some("application/vnd.github+json")
        );
        assert_eq!(hits[0].header("user-agent"), Some(crate::net::USER_AGENT));
        assert!(hits[0].body.is_empty());
    }

    #[test]
    fn no_release_yet_is_nothing_newer_not_a_failure() {
        let db = test_db();
        set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
        let net = Net::for_tests(db.reads.clone());
        let server = TestServer::start(Reply::Status(404));
        assert_eq!(latest_at(&net, &server.url("")).unwrap(), None);
    }

    #[test]
    fn a_server_error_or_a_redirect_is_an_error_for_the_caller_to_ignore() {
        let db = test_db();
        set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
        let net = Net::for_tests(db.reads.clone());
        for reply in [
            Reply::Status(500),
            Reply::Redirect("https://evil.example/".into()),
        ] {
            let server = TestServer::start(reply);
            assert!(matches!(
                latest_at(&net, &server.url("")),
                Err(ClientError::Status { .. })
            ));
            // A redirect is never followed: the server saw one request.
            assert_eq!(server.hits().len(), 1);
        }
    }

    #[test]
    fn a_reply_over_the_size_cap_is_refused() {
        let db = test_db();
        set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
        let net = Net::for_tests(db.reads.clone());
        let big = vec![b' '; Service::UpdateCheck.max_body() as usize + 1];
        let server = TestServer::start(Reply::Body(big));
        assert!(matches!(
            latest_at(&net, &server.url("")),
            Err(ClientError::Net(NetError::TooLarge { .. }))
        ));
    }

    #[test]
    fn with_update_checks_off_nothing_connects() {
        let db = test_db();
        let net = Net::for_tests(db.reads.clone());
        let server = TestServer::start(json(&release_json("v0.2.0")));
        assert!(matches!(
            latest_at(&net, &server.url("")),
            Err(ClientError::Net(NetError::NotOptedIn(Service::UpdateCheck)))
        ));
        assert!(server.hits().is_empty());
    }
}
