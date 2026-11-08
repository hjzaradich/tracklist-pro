//! Tests for the AcoustID client. Every request goes to a server on
//! 127.0.0.1 started by the test; nothing reaches the internet. The fixtures
//! are hand-made ("00000000-…" ids), never recorded replies.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use super::*;
use crate::net::tests::{local_url, test_db, Probe, Reply, TestDb, TestServer};
use crate::net::{is_opted_in, set_opt_in, NetError};

const LOOKUP: &str = include_str!("../fixtures/acoustid_lookup.json");
const ERROR: &str = include_str!("../fixtures/acoustid_error.json");

/// Never a real key.
const TEST_KEY: &str = "TESTKEY-not-a-real-key";

fn opted_in(db: &TestDb) -> Net {
    set_opt_in(&db.writer, Service::AcoustId, true).unwrap();
    Net::for_tests(db.reads.clone())
}

fn json(body: &str) -> Reply {
    Reply::Typed(200, "application/json", body.as_bytes().to_vec())
}

/// Spread-out items, so the compressed form isn't trivial.
fn items(count: usize) -> Vec<u32> {
    let mut x: u32 = 0x9E37_79B9;
    (0..count)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        })
        .collect()
}

fn blob(count: usize) -> Vec<u8> {
    Fingerprint::new(items(count)).to_blob()
}

/// The form fields of a request body.
fn form(body: &[u8]) -> Vec<(String, String)> {
    String::from_utf8(body.to_vec())
        .unwrap()
        .split('&')
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap();
            (name.to_owned(), value.to_owned())
        })
        .collect()
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("no field {name}"))
}

/// The items the server would see in a sent fingerprint.
fn sent_items(fingerprint: &str) -> Vec<u32> {
    let bytes = URL_SAFE_NO_PAD.decode(fingerprint).unwrap();
    let mut blob = b"TLFP".to_vec();
    blob.push(stored::VERSION);
    blob.extend_from_slice(&bytes);
    Fingerprint::from_blob(&blob).unwrap().items().to_vec()
}

// --- The key, the switch and the gate ---

#[test]
fn acoustid_makes_no_request_while_key_missing() {
    let db = test_db();
    let net = opted_in(&db);
    let probe = Probe::new();
    for key in ["", "  ", "\n"] {
        let result = lookup_at(&net, &local_url(probe.addr(), ""), key, &blob(100), 200);
        assert!(
            matches!(result, Err(ClientError::KeyMissing(Service::AcoustId))),
            "{key:?}: {result:?}"
        );
    }
    assert_eq!(probe.connections(), 0);
}

#[test]
fn a_missing_key_is_told_apart_from_a_key_that_is_there() {
    assert!(key_is_missing(""));
    assert!(key_is_missing("   "));
    assert!(!key_is_missing(TEST_KEY));
}

#[test]
fn acoustid_is_off_until_switched_on_and_connects_nowhere_while_off() {
    let db = test_db();
    assert!(!is_opted_in(&db.reads, Service::AcoustId).unwrap());
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    let result = lookup_at(
        &net,
        &local_url(probe.addr(), ""),
        TEST_KEY,
        &blob(100),
        200,
    );
    assert!(
        matches!(
            result,
            Err(ClientError::Net(NetError::NotOptedIn(Service::AcoustId)))
        ),
        "{result:?}"
    );
    assert_eq!(probe.connections(), 0);
}

#[test]
fn acoustid_is_pinned_to_its_own_host_and_limited_to_three_requests_a_second() {
    assert_eq!(Service::AcoustId.id(), "acoustid");
    assert_eq!(Service::AcoustId.hosts(), ["api.acoustid.org"]);
    assert!(Service::AcoustId.allows_host("api.acoustid.org"));
    assert!(!Service::AcoustId.allows_host("acoustid.org"));
    assert!(!Service::AcoustId.allows_host("api.acoustid.org.example.test"));
    assert!(!Service::MusicBrainz.allows_host("api.acoustid.org"));
    // Three requests never fit in less than a second.
    assert!(Service::AcoustId.min_interval() * 3 >= std::time::Duration::from_secs(1));
    assert!(Service::AcoustId.min_interval() < std::time::Duration::from_millis(400));
}

// --- The request ---

#[test]
fn a_lookup_posts_a_form_with_the_key_the_metadata_the_duration_and_the_fingerprint() {
    let db = test_db();
    let net = opted_in(&db);
    let server = TestServer::start(json(LOOKUP));
    lookup_at(&net, &server.url(""), TEST_KEY, &blob(300), 215).unwrap();

    let hits = server.hits();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].request_line.starts_with("POST /v2/lookup HTTP/1.1"));
    assert_eq!(
        hits[0].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert!(
        !hits[0].request_line.contains(TEST_KEY),
        "the key stays out of the URL"
    );
    let fields = form(&hits[0].body);
    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["client", "meta", "duration", "fingerprint"]);
    assert_eq!(field(&fields, "client"), TEST_KEY);
    assert_eq!(field(&fields, "meta"), "recordingids");
    assert_eq!(field(&fields, "duration"), "215");
}

#[test]
fn the_sent_fingerprint_is_the_stored_one_without_the_apps_header() {
    let db = test_db();
    let net = opted_in(&db);
    let server = TestServer::start(json(LOOKUP));
    let original = items(300);
    let stored_blob = Fingerprint::new(original.clone()).to_blob();
    assert!(stored_blob.starts_with(b"TLFP"));
    lookup_at(&net, &server.url(""), TEST_KEY, &stored_blob, 40).unwrap();

    let fields = form(&server.hits()[0].body);
    let sent = URL_SAFE_NO_PAD
        .decode(field(&fields, "fingerprint"))
        .unwrap();
    // chromaprint's own bytes: they start with the algorithm id, not "TLFP".
    assert_eq!(sent[0], stored::config().id());
    assert!(!sent.starts_with(b"TLFP"));
    // A track shorter than two minutes goes whole.
    assert_eq!(sent_items(field(&fields, "fingerprint")), original);
    // The stored blob is exactly the app's header, a version byte and these
    // same bytes.
    assert_eq!(&stored_blob[5..], &sent[..]);
}

#[test]
fn a_long_track_sends_only_its_first_two_minutes() {
    let db = test_db();
    let net = opted_in(&db);
    let server = TestServer::start(json(LOOKUP));
    // About 5 minutes of items.
    let original = items(2400);
    let stored_blob = Fingerprint::new(original.clone()).to_blob();
    lookup_at(&net, &server.url(""), TEST_KEY, &stored_blob, 300).unwrap();

    let fields = form(&server.hits()[0].body);
    let sent = sent_items(field(&fields, "fingerprint"));
    let seconds = sent.len() as f32 * stored::config().item_duration_in_seconds();
    assert!(
        (120.0..120.2).contains(&seconds),
        "{} items cover {seconds} s",
        sent.len()
    );
    assert_eq!(sent, original[..sent.len()]);
}

#[test]
fn the_sent_fingerprint_uses_only_url_safe_characters_and_no_padding() {
    for count in [1, 2, 3, 50, 97, 300, 1000] {
        let encoded = encode(&blob(count)).unwrap();
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{count}: {encoded}"
        );
    }
}

#[test]
fn a_fingerprint_that_is_not_a_stored_one_sends_nothing() {
    let db = test_db();
    let net = opted_in(&db);
    let probe = Probe::new();
    let base = local_url(probe.addr(), "");
    let good = blob(100);
    let mut damaged = good.clone();
    damaged.truncate(8);
    let empty_items = Fingerprint::new(Vec::new()).to_blob();
    for bytes in [
        &[][..],
        b"TLFP",
        b"not a fingerprint at all",
        &damaged,
        &empty_items,
    ] {
        let result = lookup_at(&net, &base, TEST_KEY, bytes, 200);
        assert!(
            matches!(result, Err(ClientError::NoFingerprint)),
            "{bytes:?}: {result:?}"
        );
    }
    // And no length.
    assert!(matches!(
        lookup_at(&net, &base, TEST_KEY, &good, 0),
        Err(ClientError::NoFingerprint)
    ));
    assert_eq!(probe.connections(), 0);
}

#[test]
fn the_duration_is_the_stored_length_in_whole_seconds_rounded_to_the_nearest() {
    assert_eq!(whole_seconds(Some(215_000)), Some(215));
    assert_eq!(whole_seconds(Some(215_499)), Some(215));
    assert_eq!(whole_seconds(Some(215_500)), Some(216));
    assert_eq!(whole_seconds(Some(1_000)), Some(1));
    assert_eq!(whole_seconds(Some(499)), None);
    assert_eq!(whole_seconds(Some(0)), None);
    assert_eq!(whole_seconds(Some(-5)), None);
    assert_eq!(whole_seconds(None), None);
}

// --- The answer ---

#[test]
fn a_lookup_returns_the_best_match_first_with_its_recording_ids() {
    let db = test_db();
    let net = opted_in(&db);
    let server = TestServer::start(json(LOOKUP));
    let matches = lookup_at(&net, &server.url(""), TEST_KEY, &blob(100), 200).unwrap();
    assert_eq!(
        matches,
        [
            AcoustidMatch {
                score: 0.97,
                recording_ids: vec![
                    "00000000-0000-4000-8000-0000000000b1".to_owned(),
                    "00000000-0000-4000-8000-0000000000b3".to_owned(),
                ],
            },
            AcoustidMatch {
                score: 0.42,
                recording_ids: vec!["00000000-0000-4000-8000-0000000000b2".to_owned()],
            },
        ]
    );
}

#[test]
fn results_with_no_recordings_are_dropped() {
    // The fixture has one with no list and one with an empty list.
    let matches = parse_lookup(LOOKUP.as_bytes()).unwrap();
    assert_eq!(matches.len(), 2);
    assert!(matches.iter().all(|m| !m.recording_ids.is_empty()));
    let none = parse_lookup(br#"{"status":"ok","results":[]}"#).unwrap();
    assert!(none.is_empty());
    let missing = parse_lookup(br#"{"status":"ok"}"#).unwrap();
    assert!(missing.is_empty());
}

#[test]
fn a_score_outside_zero_to_one_is_clamped_and_a_missing_one_drops_the_result() {
    let body = br#"{"status":"ok","results":[
        {"score": 1.7, "recordings":[{"id":"a"}]},
        {"score": -0.2, "recordings":[{"id":"b"}]},
        {"recordings":[{"id":"c"}]}
    ]}"#;
    let matches = parse_lookup(body).unwrap();
    let scores: Vec<f64> = matches.iter().map(|m| m.score).collect();
    assert_eq!(scores, [1.0, 0.0]);
}

#[test]
fn an_error_answer_becomes_a_client_error_with_its_code_and_without_its_text() {
    let db = test_db();
    let net = opted_in(&db);

    // Invalid application key (code 4), sent with status 400 as AcoustID does.
    let refused = TestServer::start(Reply::Typed(
        400,
        "application/json",
        ERROR.as_bytes().to_vec(),
    ));
    let error = lookup_at(&net, &refused.url(""), TEST_KEY, &blob(100), 200).unwrap_err();
    assert!(
        matches!(error, ClientError::Unauthorized(Service::AcoustId)),
        "{error:?}"
    );

    for (code, expected) in [(3, "ServiceError"), (14, "RateLimited")] {
        let body =
            format!(r#"{{"status":"error","error":{{"code":{code},"message":"secret detail"}}}}"#);
        let server = TestServer::start(json(&body));
        let error = lookup_at(&net, &server.url(""), TEST_KEY, &blob(100), 200).unwrap_err();
        assert!(format!("{error:?}").starts_with(expected), "{error:?}");
        let shown = format!("{error} {error:?}");
        assert!(
            !shown.contains("secret detail") && !shown.contains(TEST_KEY),
            "{shown}"
        );
    }
    let no_code = parse_lookup(br#"{"status":"error","error":{}}"#).unwrap_err();
    assert!(
        matches!(no_code, ClientError::BadReply { .. }),
        "{no_code:?}"
    );
}

#[test]
fn other_bad_answers_are_errors_not_empty_results() {
    let db = test_db();
    let net = opted_in(&db);
    let table: [(Reply, &str); 5] = [
        (Reply::Status(500), "Status"),
        (Reply::Status(429), "RateLimited"),
        (Reply::Status(403), "Unauthorized"),
        (Reply::Body(b"<html>".to_vec()), "BadReply"),
        (json(r#"{"results":[]}"#), "BadReply"),
    ];
    for (reply, expected) in table {
        let server = TestServer::start(reply);
        let error = lookup_at(&net, &server.url(""), TEST_KEY, &blob(100), 200).unwrap_err();
        assert!(format!("{error:?}").starts_with(expected), "{error:?}");
        assert!(!format!("{error} {error:?}").contains(TEST_KEY));
    }
}

#[test]
fn a_busy_answer_is_retried_as_a_new_gated_request() {
    let db = test_db();
    let net = opted_in(&db);
    let server = TestServer::start(Reply::Sequence(vec![Reply::Status(503), json(LOOKUP)]));
    let matches = lookup_at(&net, &server.url(""), TEST_KEY, &blob(100), 200).unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(server.hits().len(), 2);
    // The retry carries the same form.
    let hits = server.hits();
    assert_eq!(hits[0].body, hits[1].body);
}
