//! Tests for the lookup clients. Every request goes to a server on
//! 127.0.0.1 started by the test; nothing reaches the internet. The
//! fixtures are synthetic ("Test Artist", made-up ids), never recorded
//! replies. Live checks against the real services would be `#[ignore]`d.

use super::*;
use crate::net::tests::{local_url, test_db, Probe, Reply, TestDb, TestServer};
use crate::net::{is_opted_in, set_opt_in, Net, NetError, Service, USER_AGENT};

const MB_FIXTURE: &str = include_str!("fixtures/musicbrainz_search.json");
const MB_RECORDING: &str = include_str!("fixtures/musicbrainz_recording.json");
const MB_RELEASE: &str = include_str!("fixtures/musicbrainz_release.json");
const DISCOGS_FIXTURE: &str = include_str!("fixtures/discogs_search.json");
const DISCOGS_IDENTITY: &str = include_str!("fixtures/discogs_identity.json");

/// Never a real token.
const FAKE_TOKEN: &str = "FAKE-token-for-tests-0000";

fn opted_in(db: &TestDb, services: &[Service]) -> Net {
    for &service in services {
        set_opt_in(&db.writer, service, true).unwrap();
    }
    Net::for_tests(db.reads.clone())
}

fn json(body: &str) -> Reply {
    Reply::Typed(200, "application/json", body.as_bytes().to_vec())
}

fn image(bytes: &[u8]) -> Reply {
    Reply::Typed(200, "image/jpeg", bytes.to_vec())
}

/// The server's base URL, with no path.
fn base(server: &TestServer) -> String {
    server.url("")
}

fn is_pin_refusal(e: &ClientError) -> bool {
    matches!(e, ClientError::Net(NetError::Failed { reason, .. }) if *reason == crate::net::HOST_NOT_PINNED)
}

// --- Gate: off means off ---

#[test]
fn with_every_service_off_no_client_connects() {
    let db = test_db();
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    let base = local_url(probe.addr(), "");

    let results = [
        musicbrainz::search_at(&net, &base, "Test Artist", 5).map(drop),
        discogs::search_at(&net, &base, FAKE_TOKEN, "Test Artist", 5).map(drop),
        discogs::check_token_at(&net, &base, FAKE_TOKEN),
        coverart::front_at(&net, &base, "00000000-0000-4000-8000-0000000000b1").map(drop),
    ];
    let services = [
        Service::MusicBrainz,
        Service::Discogs,
        Service::Discogs,
        Service::CoverArt,
    ];
    for (result, service) in results.into_iter().zip(services) {
        assert!(
            matches!(result, Err(ClientError::Net(NetError::NotOptedIn(s))) if s == service),
            "{service}: {result:?}"
        );
    }
    assert_eq!(probe.connections(), 0);
}

#[test]
fn every_lookup_service_is_off_on_a_fresh_database_and_can_be_switched_on() {
    // No row means off; the table takes the new ids without a migration.
    let db = test_db();
    for service in Service::ALL {
        assert!(!is_opted_in(&db.reads, service).unwrap(), "{service}");
    }
    assert_eq!(Service::Discogs.id(), "discogs");
    assert_eq!(Service::CoverArt.id(), "cover_art");
    for service in [Service::Discogs, Service::CoverArt] {
        set_opt_in(&db.writer, service, true).unwrap();
        assert!(is_opted_in(&db.reads, service).unwrap(), "{service}");
    }
    // Each has its own switch.
    assert!(!is_opted_in(&db.reads, Service::MusicBrainz).unwrap());
}

#[test]
fn every_service_sends_the_user_agent_with_the_public_contact() {
    let db = test_db();
    let services = [Service::MusicBrainz, Service::Discogs, Service::CoverArt];
    let net = opted_in(&db, &services);
    for service in services {
        let server = TestServer::start(Reply::Body(b"ok".to_vec()));
        net.send(crate::net::Request::get(service, server.url("/")))
            .unwrap();
        assert_eq!(
            server.hits()[0].header("user-agent"),
            Some(USER_AGENT),
            "{service}"
        );
    }
    assert!(USER_AGENT.contains("https://github.com/hjzaradich/tracklist-pro"));
}

// --- MusicBrainz ---

#[test]
fn a_musicbrainz_search_sends_the_encoded_query_and_reads_the_fixture() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(json(MB_FIXTURE));

    let hits =
        musicbrainz::search_at(&net, &format!("{}/ws/2", base(&server)), "Test & Co", 5).unwrap();

    let line = server.hits()[0].request_line.clone();
    assert!(line.starts_with("GET /ws/2/recording?query=Test%20%26%20Co&limit=5&fmt=json "));
    assert_eq!(server.hits()[0].header("accept"), Some("application/json"));

    assert_eq!(hits.len(), 3);
    let full = &hits[0];
    assert_eq!(full.source, Service::MusicBrainz);
    assert_eq!(full.score, Some(100));
    assert_eq!(full.title.as_deref(), Some("Test Track"));
    assert_eq!(full.artist.as_deref(), Some("Test Artist"));
    // The first release names the album, year and release id.
    assert_eq!(full.album.as_deref(), Some("Test Album"));
    assert_eq!(full.year, Some(2001));
    assert_eq!(full.isrc.as_deref(), Some("TEST00000001"));
    assert_eq!(
        full.release_id.as_deref(),
        Some("00000000-0000-4000-8000-0000000000b1")
    );
    assert_eq!(
        full.url.as_deref(),
        Some("https://musicbrainz.org/recording/00000000-0000-4000-8000-000000000001")
    );
    assert_eq!((full.genre.as_deref(), full.label.as_deref()), (None, None));

    // Blank text, a date too short for a year and a missing isrc are `None`.
    let bare = &hits[1];
    assert_eq!(bare.title.as_deref(), Some("Bare Track"));
    assert_eq!(
        (bare.album.as_deref(), bare.year, bare.isrc.as_deref()),
        (None, None, None)
    );
    // A recording with almost nothing in it still comes back.
    let empty = &hits[2];
    assert_eq!(
        (empty.score, empty.title.as_deref(), empty.artist.as_deref()),
        (Some(10), None, None)
    );
    assert_eq!(empty.release_id, None);
}

#[test]
fn a_musicbrainz_recording_lookup_asks_for_credits_and_releases_and_reads_the_fixture() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(json(MB_RECORDING));

    let recording = musicbrainz::recording_at(
        &net,
        &format!("{}/ws/2", base(&server)),
        "00000000-0000-4000-8000-000000000001",
    )
    .unwrap();

    let line = server.hits()[0].request_line.clone();
    assert!(line.starts_with(
        "GET /ws/2/recording/00000000-0000-4000-8000-000000000001?inc=artist-credits+releases&fmt=json "
    ));
    assert_eq!(
        recording.title.as_deref(),
        Some("Test Track (Extended Mix)")
    );
    // Each credited name, then its join phrase; the credited name wins over
    // the artist's own, and an empty one falls back to it.
    assert_eq!(
        recording.artist.as_deref(),
        Some("Test Artist & Second Act feat. Third Voice")
    );
    // The release with no id is dropped; the rest keep their status and date.
    assert_eq!(recording.releases.len(), 3);
    assert_eq!(recording.releases[1].title.as_deref(), Some("Test Album"));
    assert_eq!(recording.releases[1].date.as_deref(), Some("2001-05-06"));
    assert_eq!(recording.releases[2].status.as_deref(), Some("Promotion"));
}

#[test]
fn a_musicbrainz_release_lookup_asks_for_labels_and_reads_catalogue_numbers() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(json(MB_RELEASE));

    let release = musicbrainz::release_at(
        &net,
        &format!("{}/ws/2", base(&server)),
        "00000000-0000-4000-8000-0000000000b1",
    )
    .unwrap();

    let line = server.hits()[0].request_line.clone();
    assert!(line.starts_with(
        "GET /ws/2/release/00000000-0000-4000-8000-0000000000b1?inc=labels+artist-credits&fmt=json "
    ));
    assert_eq!(release.title.as_deref(), Some("Test Album"));
    assert_eq!(release.date.as_deref(), Some("2001-05-06"));
    // A blank catalogue number is none; a label with neither part is dropped.
    assert_eq!(
        release.labels,
        vec![
            musicbrainz::LabelInfo {
                name: Some("Test Records".into()),
                catalogue_number: Some("TST001".into())
            },
            musicbrainz::LabelInfo {
                name: Some("Second Label".into()),
                catalogue_number: None
            },
        ]
    );
}

#[test]
fn a_musicbrainz_id_that_is_not_a_uuid_sends_nothing() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(json(MB_RECORDING));
    let at = format!("{}/ws/2", base(&server));
    for bad in [
        "",
        "  ",
        "..",
        "../../x?y=1",
        "%2e%2e",
        "x",
        "00000000-0000-4000-8000-000000000001/..",
        "00000000-0000-4000-8000-00000000000g",
    ] {
        assert!(
            matches!(
                musicbrainz::recording_at(&net, &at, bad),
                Err(ClientError::NoId)
            ),
            "{bad}"
        );
        assert!(matches!(
            musicbrainz::release_at(&net, &at, bad),
            Err(ClientError::NoId)
        ));
    }
    assert!(server.hits().is_empty());
}

#[test]
fn an_unknown_musicbrainz_id_is_not_found_and_junk_is_a_bad_reply() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(Reply::Typed(404, "application/json", b"{}".to_vec()));
    assert!(matches!(
        musicbrainz::recording_at(&net, &base(&server), "00000000-0000-4000-8000-000000000001"),
        Err(ClientError::NotFound(Service::MusicBrainz))
    ));
    assert!(matches!(
        musicbrainz::parse_recording(b"{}"),
        Err(ClientError::BadReply { .. })
    ));
    assert!(matches!(
        musicbrainz::parse_release(b"<html>"),
        Err(ClientError::BadReply { .. })
    ));
}

#[test]
fn a_busy_musicbrainz_reply_is_retried_through_the_gate() {
    // Each retry is a new `Net::send`, so the limiter spaces it; the
    // server's request count shows the retries happened.
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(Reply::Sequence(vec![
        Reply::Status(503),
        Reply::Status(503),
        json(MB_FIXTURE),
    ]));
    let hits = musicbrainz::search_at(&net, &base(&server), "x", 3).unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(server.hits().len(), 3);
}

#[test]
fn a_musicbrainz_that_stays_busy_is_given_up_on_after_the_retries() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(Reply::Status(503));
    let error = musicbrainz::search_at(&net, &base(&server), "x", 3).unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::Status {
                service: Service::MusicBrainz,
                status: 503
            }
        ),
        "{error:?}"
    );
    // The first try and two retries, no more.
    assert_eq!(server.hits().len(), 3);
}

#[test]
fn other_statuses_and_junk_are_errors_not_empty_results() {
    let db = test_db();
    let net = opted_in(&db, &[Service::MusicBrainz]);
    let server = TestServer::start(Reply::Status(404));
    let result = musicbrainz::search_at(&net, &base(&server), "x", 3);
    assert!(matches!(
        result,
        Err(ClientError::NotFound(Service::MusicBrainz))
    ));
    // Not retried.
    assert_eq!(server.hits().len(), 1);

    assert!(matches!(
        musicbrainz::parse_search(b"<html>"),
        Err(ClientError::BadReply { .. })
    ));
    // A JSON reply with no recordings is an empty list.
    assert_eq!(musicbrainz::parse_search(b"{}").unwrap(), Vec::new());
}

// --- Discogs ---

#[test]
fn a_discogs_search_reads_the_fixture() {
    let hits = discogs::parse_search(DISCOGS_FIXTURE.as_bytes()).unwrap();
    assert_eq!(hits.len(), 3);

    let first = &hits[0];
    assert_eq!(first.source, Service::Discogs);
    // "Artist - Title" is split, the year comes as a string, and the more
    // specific style wins over the genre.
    assert_eq!(first.artist.as_deref(), Some("Test Artist"));
    assert_eq!(first.title.as_deref(), Some("Test Track"));
    assert_eq!(first.year, Some(2003));
    assert_eq!(first.genre.as_deref(), Some("Deep House"));
    assert_eq!(first.label.as_deref(), Some("Test Label"));
    assert_eq!(first.catalogue_number.as_deref(), Some("TL-001"));
    assert_eq!(
        first.url.as_deref(),
        Some("https://www.discogs.com/release/1-test-track")
    );
    assert_eq!(
        (first.score, first.album.as_deref(), first.isrc.as_deref()),
        (None, None, None)
    );

    // Only the first " - " splits; a number is a year too; no style means
    // the genre; an empty label is none.
    let second = &hits[1];
    assert_eq!(second.artist.as_deref(), Some("Test Artist"));
    assert_eq!(second.title.as_deref(), Some("Second - Remix"));
    assert_eq!(second.year, Some(1999));
    assert_eq!(second.genre.as_deref(), Some("Electronic"));
    assert_eq!(second.label, None);

    let third = &hits[2];
    assert_eq!(third.artist, None);
    assert_eq!(third.title.as_deref(), Some("No Separator Here"));
    assert_eq!(third.year, None);
    assert_eq!((third.genre.as_deref(), third.url.as_deref()), (None, None));
}

#[test]
fn a_discogs_search_sends_the_token_header_and_the_encoded_query() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);
    let server = TestServer::start(json(DISCOGS_FIXTURE));

    let hits = discogs::search_at(
        &net,
        &base(&server),
        &format!("  {FAKE_TOKEN} "),
        "Test Artist",
        500,
    )
    .unwrap();
    assert_eq!(hits.len(), 3);

    let hit = &server.hits()[0];
    assert_eq!(
        hit.header("authorization"),
        Some(format!("Discogs token={FAKE_TOKEN}").as_str())
    );
    // The page size is capped.
    assert!(hit
        .request_line
        .starts_with("GET /database/search?q=Test%20Artist&type=release&per_page=100 "));
}

#[test]
fn discogs_without_a_usable_token_sends_nothing() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);
    let probe = Probe::new();
    for token in ["", "   ", "abc\r\nX-Evil: 1"] {
        let result = discogs::search_at(&net, &local_url(probe.addr(), ""), token, "x", 5);
        assert!(
            matches!(result, Err(ClientError::NoToken)),
            "{token:?}: {result:?}"
        );
    }
    assert_eq!(probe.connections(), 0);
}

#[test]
fn a_rejected_discogs_token_is_reported_without_the_token() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);
    let server = TestServer::start(Reply::Status(401));
    let error = discogs::search_at(&net, &base(&server), FAKE_TOKEN, "x", 5).unwrap_err();
    assert!(
        matches!(error, ClientError::Unauthorized(Service::Discogs)),
        "{error:?}"
    );
    assert!(!error.to_string().contains(FAKE_TOKEN));
    assert!(!format!("{error:?}").contains(FAKE_TOKEN));
}

#[test]
fn checking_a_discogs_token_wants_an_account_in_the_reply() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);

    let good = TestServer::start(json(DISCOGS_IDENTITY));
    discogs::check_token_at(&net, &base(&good), FAKE_TOKEN).unwrap();
    assert!(good.hits()[0]
        .request_line
        .starts_with("GET /oauth/identity "));

    let anonymous = TestServer::start(json("{}"));
    let result = discogs::check_token_at(&net, &base(&anonymous), FAKE_TOKEN);
    assert!(
        matches!(result, Err(ClientError::BadReply { .. })),
        "{result:?}"
    );
}

#[test]
fn the_discogs_token_never_reaches_a_host_the_service_doesnt_own() {
    // Only 127.0.0.1 stands in for Discogs here. The same server is also
    // reachable as `localhost`, which isn't pinned: the request must be
    // refused, and the token appear nowhere in the error.
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);
    let server = TestServer::start(json(DISCOGS_FIXTURE));
    let elsewhere = format!("http://localhost:{}", server.addr.port());

    let error = discogs::search_at(&net, &elsewhere, FAKE_TOKEN, "x", 5).unwrap_err();
    assert!(is_pin_refusal(&error), "{error:?}");
    assert!(!error.to_string().contains(FAKE_TOKEN));
    assert!(!format!("{error:?}").contains(FAKE_TOKEN));
    assert!(server.hits().is_empty());

    // And with the app's real pinning, not even loopback is Discogs.
    let strict = Net::for_tests_pinned(db.reads.clone());
    let error = discogs::search_at(&strict, &base(&server), FAKE_TOKEN, "x", 5).unwrap_err();
    assert!(is_pin_refusal(&error), "{error:?}");
    assert!(server.hits().is_empty());
}

#[test]
fn a_discogs_redirect_is_not_followed_and_its_target_gets_no_token() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);
    let target = TestServer::start(Reply::Body(b"{}".to_vec()));
    let server = TestServer::start(Reply::Redirect(target.url("/steal")));

    let error = discogs::search_at(&net, &base(&server), FAKE_TOKEN, "x", 5).unwrap_err();
    assert!(
        matches!(error, ClientError::Status { status: 302, .. }),
        "{error:?}"
    );
    assert_eq!(server.hits().len(), 1);
    assert!(target.hits().is_empty());
}

// --- Cover Art Archive ---

const RELEASE: &str = "00000000-0000-4000-8000-0000000000b1";

#[test]
fn a_cover_comes_back_with_its_bytes_and_type() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let server = TestServer::start(image(b"\xFF\xD8test-image"));
    let art = coverart::front_at(&net, &base(&server), RELEASE)
        .unwrap()
        .unwrap();
    assert_eq!(art.bytes, b"\xFF\xD8test-image");
    assert_eq!(art.mime, "image/jpeg");
    assert!(server.hits()[0]
        .request_line
        .starts_with(&format!("GET /release/{RELEASE}/front ")));
}

#[test]
fn no_cover_is_none_not_an_error() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let missing = TestServer::start(Reply::Status(404));
    assert_eq!(
        coverart::front_at(&net, &base(&missing), RELEASE).unwrap(),
        None
    );
    let empty = TestServer::start(image(b""));
    assert_eq!(
        coverart::front_at(&net, &base(&empty), RELEASE).unwrap(),
        None
    );
}

#[test]
fn a_cover_reply_that_isnt_an_image_is_an_error() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let server = TestServer::start(Reply::Typed(200, "text/html", b"<html>".to_vec()));
    let result = coverart::front_at(&net, &base(&server), RELEASE);
    assert!(
        matches!(result, Err(ClientError::BadReply { .. })),
        "{result:?}"
    );
}

#[test]
fn a_cover_over_the_size_limit_is_an_error() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let limit = Service::CoverArt.max_body();
    let server = TestServer::start(image(&vec![0u8; limit as usize + 1]));
    let result = coverart::front_at(&net, &base(&server), RELEASE);
    assert!(
        matches!(
            result,
            Err(ClientError::Net(NetError::TooLarge {
                service: Service::CoverArt,
                ..
            }))
        ),
        "{result:?}"
    );
}

#[test]
fn asking_for_a_cover_without_an_id_sends_nothing() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let probe = Probe::new();
    for id in ["", "   "] {
        let result = coverart::front_at(&net, &local_url(probe.addr(), ""), id);
        assert!(
            matches!(result, Err(ClientError::NoId)),
            "{id:?}: {result:?}"
        );
    }
    assert_eq!(probe.connections(), 0);
}

#[test]
fn a_release_id_that_is_not_a_uuid_sends_nothing() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let server = TestServer::start(Reply::Status(404));
    for bad in [
        "",
        "..",
        "../x?y=1",
        "%2e%2e",
        "x",
        "00000000-0000-4000-8000-0000000000b1/../x",
    ] {
        assert!(
            matches!(
                coverart::front_at(&net, &base(&server), bad),
                Err(ClientError::NoId)
            ),
            "{bad}"
        );
    }
    assert!(server.hits().is_empty());
}

#[test]
fn the_archives_redirect_is_followed_through_the_gate() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let host = TestServer::start(image(b"test-image"));
    let archive = TestServer::start(Reply::Redirect(host.url("/files/cover.jpg")));

    let art = coverart::front_at(&net, &base(&archive), RELEASE)
        .unwrap()
        .unwrap();
    assert_eq!(art.bytes, b"test-image");
    assert_eq!(archive.hits().len(), 1);
    assert!(host.hits()[0]
        .request_line
        .starts_with("GET /files/cover.jpg "));
    assert_eq!(host.hits()[0].header("user-agent"), Some(USER_AGENT));
}

#[test]
fn a_redirect_to_a_host_cover_art_doesnt_own_is_refused() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let server = TestServer::start(image(b"test-image"));
    // Same server, but under a name that isn't pinned.
    let elsewhere = format!("http://localhost:{}/files/cover.jpg", server.addr.port());
    let archive = TestServer::start(Reply::Redirect(elsewhere));

    let error = coverart::front_at(&net, &base(&archive), RELEASE).unwrap_err();
    assert!(is_pin_refusal(&error), "{error:?}");
    assert!(server.hits().is_empty());
}

#[test]
fn a_redirect_loop_stops_after_a_few_hops() {
    let db = test_db();
    let net = opted_in(&db, &[Service::CoverArt]);
    let server = TestServer::start(Reply::Redirect("/again".to_owned()));
    let result = coverart::front_at(&net, &base(&server), RELEASE);
    assert!(
        matches!(result, Err(ClientError::BadReply { .. })),
        "{result:?}"
    );
    // The first request and three hops, no more.
    assert_eq!(server.hits().len(), 4);
}

// --- Shared helpers ---

#[test]
fn urlencode_escapes_everything_but_unreserved_characters() {
    let table = [
        ("Test Artist", "Test%20Artist"),
        ("AC/DC", "AC%2FDC"),
        ("Sigur Rós", "Sigur%20R%C3%B3s"),
        ("a&b=c", "a%26b%3Dc"),
        ("plain-text_1.0~", "plain-text_1.0~"),
        ("", ""),
        ("100%", "100%25"),
        ("a+b", "a%2Bb"),
    ];
    for (input, expected) in table {
        assert_eq!(urlencode(input), expected, "{input:?}");
    }
}

#[test]
fn a_search_size_is_kept_between_one_and_the_most_the_services_allow() {
    assert_eq!(clamp_limit(0), 1);
    assert_eq!(clamp_limit(25), 25);
    assert_eq!(clamp_limit(10_000), MAX_LIMIT);
}

#[test]
fn client_errors_never_carry_a_url() {
    let errors = [
        ClientError::Net(NetError::Failed {
            host: Some("musicbrainz.org".to_owned()),
            reason: "it timed out",
        }),
        ClientError::NotFound(Service::CoverArt),
        ClientError::Status {
            service: Service::Discogs,
            status: 500,
        },
        ClientError::BadReply {
            service: Service::MusicBrainz,
            what: "it wasn't JSON",
        },
        ClientError::NoToken,
        ClientError::NoId,
    ];
    for error in errors {
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("://") && !shown.contains('?'), "{shown}");
    }
}

#[test]
fn printing_a_discogs_request_does_not_show_the_token() {
    let request = crate::net::Request::get(Service::Discogs, "https://api.discogs.com/x?q=a")
        .header("Authorization", "Discogs token=fake-token-123");
    let shown = format!("{request:?}");
    assert!(!shown.contains("fake-token-123"));
    assert!(shown.contains("Authorization") && shown.contains("api.discogs.com"));
}

#[test]
fn a_discogs_search_never_requests_an_image_though_the_reply_names_them() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs, Service::CoverArt]);
    // The fixture's first result names a thumbnail and a cover image.
    assert!(DISCOGS_FIXTURE.contains("cover_image") && DISCOGS_FIXTURE.contains("thumb"));
    let server = TestServer::start(json(DISCOGS_FIXTURE));
    let hits = discogs::search_at(&net, &base(&server), FAKE_TOKEN, "Test Artist", 10).unwrap();
    assert_eq!(hits.len(), 3);
    // One request, the search; no address from the reply was followed.
    let seen = server.hits();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].request_line.starts_with("GET /database/search?"));
    // And the hits carry no image address.
    assert!(hits.iter().all(|h| !format!("{h:?}").contains(".jpg")));
}

const DISCOGS_TRACK_FIXTURE: &str = include_str!("fixtures/discogs_track_search.json");

#[test]
fn a_discogs_track_search_asks_for_releases_containing_the_track_and_reads_the_release() {
    let db = test_db();
    let net = opted_in(&db, &[Service::Discogs]);
    let server = TestServer::start(json(DISCOGS_TRACK_FIXTURE));
    let hits = discogs::search_track_at(
        &net,
        &base(&server),
        FAKE_TOKEN,
        "Test Artist",
        "Test Track",
        10,
    )
    .unwrap();
    // One request, by the artist and track fields, releases only.
    let seen = server.hits();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].request_line.starts_with(
            "GET /database/search?artist=Test%20Artist&track=Test%20Track&type=release&per_page=10 "
        ),
        "{}",
        seen[0].request_line
    );
    assert_eq!(
        seen[0].header("authorization"),
        Some(format!("Discogs token={FAKE_TOKEN}").as_str())
    );
    // The hit is the release: its title is not the track's.
    assert_eq!(hits[0].artist.as_deref(), Some("Test Artist"));
    assert_eq!(hits[0].title.as_deref(), Some("Test Album"));
    assert_eq!(hits[0].catalogue_number.as_deref(), Some("TL-001"));
    assert_eq!(hits[0].label.as_deref(), Some("Test Label"));
    assert_eq!(hits[0].year, Some(2003));
    assert_eq!(hits[1].artist.as_deref(), Some("Various"));
}
