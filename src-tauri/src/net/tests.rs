//! Tests for the network gate. Every request here goes to a server on
//! 127.0.0.1 started by the test; nothing reaches the internet.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::db::Writer;

impl Net {
    /// The gate with its own rate limiter, so tests don't wait on each
    /// other, and plain HTTP allowed, for the local test server.
    fn for_tests(reads: ReadPool) -> Net {
        Net {
            reads,
            agent: agent(false),
            limiter: Arc::default(),
            timeout_override: None,
        }
    }
}

/// A fresh database in a temp folder. Keep it alive as long as the handles.
struct TestDb {
    _dir: tempfile::TempDir,
    writer: Writer,
    reads: ReadPool,
}

fn test_db() -> TestDb {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(dir.path(), "test.db")).unwrap();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    TestDb {
        _dir: dir,
        writer,
        reads,
    }
}

/// A URL on the local test server. Refuses anything but loopback, so a
/// test can't reach the internet by mistake.
fn local_url(addr: SocketAddr, path: &str) -> String {
    assert!(addr.ip().is_loopback(), "tests only talk to 127.0.0.1");
    format!("http://{addr}{path}")
}

/// A listening socket that never answers. It counts connection attempts:
/// a connect finishes in the OS before the app would send anything, so an
/// attempt shows up here even if the request was abandoned.
struct Probe {
    listener: TcpListener,
}

impl Probe {
    fn new() -> Probe {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        Probe { listener }
    }

    fn addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap()
    }

    /// How many connections were attempted since the last call.
    fn connections(&self) -> usize {
        let mut n = 0;
        loop {
            match self.listener.accept() {
                Ok(_) => n += 1,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return n,
                Err(e) => panic!("probe: {e}"),
            }
        }
    }
}

/// What the test server answers.
#[derive(Clone)]
enum Reply {
    /// 200 with this body.
    Body(Vec<u8>),
    /// This status, empty body.
    Status(u16),
    /// 302 to this location.
    Redirect(String),
    /// Reads the request and never answers.
    Silent,
}

/// A request the test server received.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Hit {
    at: Instant,
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Hit {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A tiny HTTP/1.1 server on 127.0.0.1 that records every request.
struct TestServer {
    addr: SocketAddr,
    hits: Arc<Mutex<Vec<Hit>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(reply: Reply) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (hits, stop) = (Arc::clone(&hits), Arc::clone(&stop));
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    let (hits, reply) = (Arc::clone(&hits), reply.clone());
                    thread::spawn(move || serve(stream, &hits, &reply));
                }
            })
        };
        TestServer {
            addr,
            hits,
            stop,
            thread: Some(thread),
        }
    }

    fn url(&self, path: &str) -> String {
        local_url(self.addr, path)
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees `stop`.
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(mut stream: TcpStream, hits: &Mutex<Vec<Hit>>, reply: &Reply) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return, // e.g. the stop connection
            Ok(n) => data.extend_from_slice(&buf[..n]),
        }
    };
    let at = Instant::now();
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = data[head_end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
        }
    }
    hits.lock().unwrap().push(Hit {
        at,
        request_line,
        headers,
        body,
    });
    let mut extra = String::new();
    let (status, body) = match reply {
        Reply::Body(b) => (200, b.clone()),
        Reply::Status(s) => (*s, Vec::new()),
        Reply::Redirect(location) => {
            extra = format!("Location: {location}\r\n");
            (302, Vec::new())
        }
        Reply::Silent => {
            thread::sleep(Duration::from_secs(10));
            return;
        }
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: text/plain; charset=utf-8\r\n{extra}\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

fn ok_server() -> TestServer {
    TestServer::start(Reply::Body(b"ok".to_vec()))
}

// --- Refused: nothing is sent, no connection is attempted ---

#[test]
fn the_probe_sees_a_connection_attempt() {
    // Proves the zero counts below mean something.
    let probe = Probe::new();
    assert_eq!(probe.connections(), 0);
    let _stream = TcpStream::connect(probe.addr()).unwrap();
    // `connect` can return before the connection reaches the listener's
    // accept queue (seen on macOS), so poll for it instead of asserting at
    // once.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = probe.connections();
    while seen == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
        seen += probe.connections();
    }
    assert_eq!(seen, 1, "the probe didn't see the connection within 2 s");
}

#[test]
fn refused_when_the_service_has_no_opt_in_row_and_nothing_connects() {
    let db = test_db();
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    for service in Service::ALL {
        let result = net.send(Request::get(service, local_url(probe.addr(), "/")));
        assert!(
            matches!(result, Err(NetError::NotOptedIn(s)) if s == service),
            "{service}: {result:?}"
        );
    }
    assert_eq!(probe.connections(), 0);
}

#[test]
fn refused_when_opted_out_and_nothing_connects() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, false).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    let result = net.send(Request::post(
        Service::MusicBrainz,
        local_url(probe.addr(), "/ws/2/recording"),
        b"data".to_vec(),
    ));
    assert!(matches!(
        result,
        Err(NetError::NotOptedIn(Service::MusicBrainz))
    ));
    assert_eq!(probe.connections(), 0);
}

#[test]
fn opting_in_to_one_service_does_not_open_another() {
    let db = test_db();
    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    for service in [Service::MusicBrainz, Service::ModelDownload] {
        let result = net.send(Request::get(service, local_url(probe.addr(), "/")));
        assert!(matches!(result, Err(NetError::NotOptedIn(s)) if s == service));
    }
    assert_eq!(probe.connections(), 0);
}

#[test]
fn an_unreadable_opt_in_is_refused_and_nothing_connects() {
    // A database without the service_optin table: the gate can't read the
    // opt-in, so it fails closed.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bare.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let net = Net::for_tests(
        ReadPool::open(&crate::write_guard::test_path(dir.path(), "bare.db")).unwrap(),
    );
    let probe = Probe::new();
    let result = net.send(Request::get(
        Service::MusicBrainz,
        local_url(probe.addr(), "/"),
    ));
    assert!(
        matches!(result, Err(NetError::OptInUnreadable(_))),
        "{result:?}"
    );
    assert_eq!(probe.connections(), 0);
}

#[test]
fn a_refused_request_does_not_use_up_a_turn() {
    let db = test_db();
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    let _ = net.send(Request::get(
        Service::MusicBrainz,
        local_url(probe.addr(), "/"),
    ));
    assert_eq!(net.limiter.next_slot(Service::MusicBrainz), None);
}

#[test]
fn the_real_gate_refuses_plain_http_without_connecting() {
    // `Net::new` is what the app uses: HTTPS only, even when opted in.
    let db = test_db();
    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    let net = Net::new(db.reads.clone());
    let probe = Probe::new();
    let result = net.send(Request::get(
        Service::UpdateCheck,
        local_url(probe.addr(), "/"),
    ));
    assert!(matches!(result, Err(NetError::Failed { .. })), "{result:?}");
    assert_eq!(probe.connections(), 0);
}

// --- Allowed ---

#[test]
fn allowed_when_opted_in() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = ok_server();

    let response = net
        .send(Request::get(
            Service::MusicBrainz,
            server.url("/ws/2/artist?query=x"),
        ))
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"ok");
    assert_eq!(response.content_type.as_deref(), Some("text/plain"));
    let hits = server.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].request_line, "GET /ws/2/artist?query=x HTTP/1.1");
}

#[test]
fn every_request_carries_the_user_agent() {
    let db = test_db();
    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = ok_server();
    net.send(Request::get(Service::UpdateCheck, server.url("/")))
        .unwrap();
    assert_eq!(server.hits()[0].header("user-agent"), Some(USER_AGENT));
}

#[test]
fn the_user_agent_names_the_app_its_version_and_a_contact() {
    // MusicBrainz's format: `Application/version ( contact )`.
    let expected = format!(
        "tracklist-pro/{} ( https://github.com/hjzaradich/tracklist-pro )",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(USER_AGENT, expected);
}

#[test]
fn a_post_sends_its_body_and_headers() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = ok_server();
    net.send(
        Request::post(Service::MusicBrainz, server.url("/submit"), b"{}".to_vec())
            .header("Authorization", "Token users-own-key"),
    )
    .unwrap();
    let hit = &server.hits()[0];
    assert_eq!(hit.request_line, "POST /submit HTTP/1.1");
    assert_eq!(hit.body, b"{}");
    assert_eq!(hit.header("authorization"), Some("Token users-own-key"));
}

#[test]
fn error_statuses_come_back_as_responses() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = TestServer::start(Reply::Status(503));
    let response = net
        .send(Request::get(Service::MusicBrainz, server.url("/")))
        .unwrap();
    assert_eq!(response.status, 503);
}

#[test]
fn a_body_over_the_services_limit_is_an_error() {
    let db = test_db();
    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let limit = Service::UpdateCheck.max_body();
    let server = TestServer::start(Reply::Body(vec![b'x'; limit as usize + 1]));
    let result = net.send(Request::get(Service::UpdateCheck, server.url("/")));
    assert!(
        matches!(result, Err(NetError::TooLarge { service: Service::UpdateCheck, limit: l }) if l == limit),
        "{result:?}"
    );
}

#[test]
fn a_service_that_never_answers_times_out() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let mut net = Net::for_tests(db.reads.clone());
    net.timeout_override = Some(Duration::from_millis(300));
    let server = TestServer::start(Reply::Silent);
    let started = Instant::now();
    let result = net.send(Request::get(Service::MusicBrainz, server.url("/")));
    assert!(matches!(result, Err(NetError::Failed { .. })), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn every_service_has_a_timeout_and_a_body_limit() {
    for service in Service::ALL {
        assert!(!service.timeout().is_zero(), "{service}");
        assert!(service.max_body() > 0, "{service}");
    }
    assert!(!CONNECT_TIMEOUT.is_zero());
}

// --- Opting out takes effect at once ---

#[test]
fn opting_out_takes_effect_at_once() {
    let db = test_db();
    let net = Net::for_tests(db.reads.clone());
    let server = ok_server();
    let get = || net.send(Request::get(Service::UpdateCheck, server.url("/")));

    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    get().unwrap();
    set_opt_in(&db.writer, Service::UpdateCheck, false).unwrap();
    assert!(matches!(get(), Err(NetError::NotOptedIn(_))));
    assert_eq!(server.hits().len(), 1);

    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    get().unwrap();
    assert_eq!(server.hits().len(), 2);
}

#[test]
fn opting_out_stops_a_request_already_waiting_its_turn() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = ok_server();
    let url = server.url("/");

    // The first request takes this second's turn.
    net.send(Request::get(Service::MusicBrainz, &url)).unwrap();
    let first_next = net.limiter.next_slot(Service::MusicBrainz).unwrap();

    // The second passes the gate and books the next turn, a second away.
    let waiting = {
        let (net, url) = (net.clone(), url.clone());
        thread::spawn(move || net.send(Request::get(Service::MusicBrainz, url)))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while net.limiter.next_slot(Service::MusicBrainz) == Some(first_next) {
        assert!(
            Instant::now() < deadline,
            "the second request never booked a turn"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        Instant::now() < first_next,
        "opting out must happen during the wait"
    );

    set_opt_in(&db.writer, Service::MusicBrainz, false).unwrap();
    let result = waiting.join().unwrap();
    assert!(
        matches!(result, Err(NetError::NotOptedIn(Service::MusicBrainz))),
        "{result:?}"
    );
    assert_eq!(server.hits().len(), 1);
}

// --- Rate limits ---

#[test]
fn musicbrainz_allows_one_request_per_second() {
    assert_eq!(Service::MusicBrainz.min_interval(), Duration::from_secs(1));
}

#[test]
fn musicbrainz_requests_reach_the_server_at_least_a_second_apart() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = ok_server();
    let url = server.url("/");

    // Capture start time before spawning: three requests at 1 s spacing
    // means the limiter books the next slot at start + 3 s.
    let start = Instant::now();
    let threads: Vec<_> = (0..3)
        .map(|_| {
            let (net, url) = (net.clone(), url.clone());
            thread::spawn(move || net.send(Request::get(Service::MusicBrainz, url)))
        })
        .collect();
    for t in threads {
        t.join().unwrap().unwrap();
    }

    // Three requests reached the server.
    assert_eq!(server.hits().len(), 3);
    // The limiter must have booked the next turn at least 3 intervals (3 seconds)
    // from start. This proves Net::send() called limiter.wait() and sending().
    let next = net.limiter.next_slot(Service::MusicBrainz).unwrap();
    assert!(
        next >= start + Duration::from_secs(3),
        "limiter's next slot should be at least 3 s from start: start={:?}, next={:?}",
        start,
        next
    );
}

#[test]
fn the_limiter_books_turns_exactly_the_interval_apart() {
    let limiter = RateLimiter::default();
    let interval = Duration::from_millis(250);
    let slots: Vec<Instant> = (0..5)
        .map(|_| limiter.reserve(Service::MusicBrainz, interval))
        .collect();
    for pair in slots.windows(2) {
        assert_eq!(pair[1] - pair[0], interval);
    }
    // Other services have their own turns.
    let other = limiter.reserve(Service::UpdateCheck, interval);
    assert!(other < slots[1]);
}

#[test]
fn the_limiter_starts_fresh_after_a_quiet_spell() {
    let limiter = RateLimiter::default();
    let interval = Duration::from_millis(20);
    limiter.reserve(Service::MusicBrainz, interval);
    thread::sleep(Duration::from_millis(50));
    let before = Instant::now();
    let slot = limiter.reserve(Service::MusicBrainz, interval);
    assert!(slot >= before, "a turn is never booked in the past");
}

#[test]
fn the_limiter_spaces_waiters_on_many_threads() {
    let limiter = Arc::new(RateLimiter::default());
    let interval = Duration::from_millis(100);
    let start = Instant::now();
    let done = Arc::new(Mutex::new(Vec::new()));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (limiter, done) = (Arc::clone(&limiter), Arc::clone(&done));
            thread::spawn(move || {
                limiter.wait(Service::MusicBrainz, interval);
                done.lock().unwrap().push(Instant::now());
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let mut done = done.lock().unwrap().clone();
    done.sort();
    // Turns are `interval` apart and no waiter leaves before its turn, so
    // the i-th to leave left at least i intervals after the start.
    for (i, at) in done.iter().enumerate() {
        assert!(*at - start >= interval * i as u32, "waiter {i} left early");
    }
}

#[test]
fn every_net_handle_in_the_app_shares_one_limiter() {
    let db = test_db();
    let a = Net::new(db.reads.clone());
    let b = Net::new(db.reads.clone());
    assert!(Arc::ptr_eq(&a.limiter, &b.limiter));
}

// --- Opt-in storage ---

#[test]
fn every_service_id_fits_the_table_and_round_trips() {
    let db = test_db();
    let mut ids: Vec<&str> = Service::ALL.iter().map(|s| s.id()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), Service::ALL.len(), "service ids must be unique");
    for service in Service::ALL {
        assert!(
            !is_opted_in(&db.reads, service).unwrap(),
            "{service} is on by default"
        );
        // The table's CHECK rejects a malformed id.
        set_opt_in(&db.writer, service, true).unwrap();
        assert!(is_opted_in(&db.reads, service).unwrap());
        set_opt_in(&db.writer, service, false).unwrap();
        assert!(!is_opted_in(&db.reads, service).unwrap());
    }
}

#[test]
fn opting_in_records_when_and_opting_out_clears_it() {
    let db = test_db();
    let enabled_at = || {
        db.reads
            .read(|c| {
                c.query_row(
                    "SELECT enabled_at FROM service_optin WHERE service = 'musicbrainz'",
                    [],
                    |r| r.get::<_, Option<String>>(0),
                )
            })
            .unwrap()
    };
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let first = enabled_at().expect("opting in records the time");
    thread::sleep(Duration::from_millis(5));
    // Opting in again keeps the original time.
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    assert_eq!(enabled_at(), Some(first));
    set_opt_in(&db.writer, Service::MusicBrainz, false).unwrap();
    assert_eq!(enabled_at(), None);
}

// --- Redirects are never followed ---

#[test]
fn a_redirect_comes_back_and_its_target_is_never_contacted() {
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    // The second host, which must see nothing: not the request, not the key.
    let elsewhere = Probe::new();
    let server = TestServer::start(Reply::Redirect(local_url(elsewhere.addr(), "/steal")));

    let response = net
        .send(
            Request::get(Service::MusicBrainz, server.url("/ws/2/artist"))
                .header("X-Api-Key", "users-own-key"),
        )
        .unwrap();

    assert_eq!(response.status, 302);
    assert_eq!(server.hits().len(), 1);
    assert_eq!(elsewhere.connections(), 0);
}

#[test]
fn a_self_redirecting_server_gets_exactly_one_hit_per_send() {
    // A redirect hop would be a request that skipped the rate limiter.
    let db = test_db();
    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    let net = Net::for_tests(db.reads.clone());
    let server = TestServer::start(Reply::Redirect("/again".to_owned()));
    for sends in 1..=2 {
        let response = net
            .send(Request::get(Service::UpdateCheck, server.url("/")))
            .unwrap();
        assert_eq!(response.status, 302);
        assert_eq!(server.hits().len(), sends);
    }
}

// --- Errors never carry the URL ---

#[test]
fn errors_name_the_host_but_never_the_urls_secrets() {
    const SECRET: &str = "S3CRET-token-value";
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let mut net = Net::for_tests(db.reads.clone());
    net.timeout_override = Some(Duration::from_millis(300));
    let silent = TestServer::start(Reply::Silent);
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    }; // dropped: nothing listens there now
    let strict = Net::new(db.reads.clone());

    let cases = [
        (&net, local_url(silent.addr, &format!("/x?token={SECRET}"))),
        (&net, local_url(closed, &format!("/x?token={SECRET}"))),
        (
            &net,
            format!("http://user:{SECRET}@127.0.0.1:{}/x", closed.port()),
        ),
        (&net, format!("http://127.0.0.1:1/bad path?token={SECRET}")),
        // The real gate: HTTPS only.
        (&strict, local_url(closed, &format!("/x?token={SECRET}"))),
    ];
    for (net, url) in cases {
        let error = net
            .send(Request::get(Service::MusicBrainz, url.clone()))
            .unwrap_err();
        let (shown, debug) = (error.to_string(), format!("{error:?}"));
        assert!(matches!(error, NetError::Failed { .. }), "{url}: {debug}");
        assert!(!shown.contains(SECRET), "{url}: {shown}");
        assert!(!debug.contains(SECRET), "{url}: {debug}");
        assert!(!shown.contains("/x"), "no path either: {shown}");
    }
    // It still says where.
    let error = net
        .send(Request::get(
            Service::MusicBrainz,
            local_url(closed, &format!("/x?token={SECRET}")),
        ))
        .unwrap_err();
    assert!(error.to_string().contains("127.0.0.1"), "{error}");
}

#[test]
fn the_next_turn_counts_from_when_a_request_actually_went_out() {
    // A request whose opt-in read was slow goes out late; the next one must
    // still wait a full interval after it.
    let limiter = RateLimiter::default();
    let interval = Duration::from_millis(100);
    limiter.reserve(Service::MusicBrainz, interval);
    thread::sleep(Duration::from_millis(60));
    let sent = Instant::now();
    limiter.sending(Service::MusicBrainz, interval);
    let next = limiter.reserve(Service::MusicBrainz, interval);
    assert!(
        next >= sent + interval,
        "next turn only {:?} after",
        next - sent
    );
}

#[test]
fn marking_a_send_never_moves_a_booked_turn_earlier() {
    let limiter = RateLimiter::default();
    let interval = Duration::from_millis(100);
    for _ in 0..3 {
        limiter.reserve(Service::MusicBrainz, interval);
    }
    let booked = limiter.next_slot(Service::MusicBrainz).unwrap();
    limiter.sending(Service::MusicBrainz, interval);
    assert_eq!(limiter.next_slot(Service::MusicBrainz), Some(booked));
}

#[test]
fn a_request_held_up_after_its_turn_still_pushes_the_next_one_back() {
    // One read connection, so holding it makes the gate's second opt-in
    // check slow: the request goes out well after its turn came.
    // Verify that sending() (called in Net::send) pushes the next turn back,
    // even if the request was delayed.
    let db = test_db();
    set_opt_in(&db.writer, Service::MusicBrainz, true).unwrap();
    let reads = ReadPool::with_size(db.writer.guarded_path(), 1).unwrap();
    let net = Net::for_tests(reads.clone());
    let server = ok_server();
    let url = server.url("/");

    // First request: immediate.
    net.send(Request::get(Service::MusicBrainz, &url)).unwrap();
    let first_next = net.limiter.next_slot(Service::MusicBrainz).unwrap();

    // Second request starts, books a turn, then gets blocked on opt-in read.
    let second = {
        let (net, url) = (net.clone(), url.clone());
        thread::spawn(move || net.send(Request::get(Service::MusicBrainz, url)))
    };

    // Wait for second to book its turn.
    let deadline = Instant::now() + Duration::from_secs(10);
    while net.limiter.next_slot(Service::MusicBrainz) == Some(first_next) {
        assert!(
            Instant::now() < deadline,
            "second request never booked a turn"
        );
        thread::sleep(Duration::from_millis(5));
    }

    // Record the slot before the read hold.
    let slot_before_hold = net.limiter.next_slot(Service::MusicBrainz).unwrap();

    // Hold the read connection long enough to overlap the next turn.
    // Second is at slot_before_hold; the next turn is at slot_before_hold + interval.
    // Hold for 1.5s to ensure the hold covers the next turn arrival time.
    reads
        .read(|_| {
            thread::sleep(Duration::from_millis(1500));
            Ok(())
        })
        .unwrap();

    // Second finishes, calls sending(), which should push the next slot forward.
    second.join().unwrap().unwrap();

    let slot_after = net.limiter.next_slot(Service::MusicBrainz).unwrap();
    // The second request's sending() call should have pushed the next slot back.
    assert!(
        slot_after > slot_before_hold,
        "sending() should push next slot forward: before={:?}, after={:?}",
        slot_before_hold,
        slot_after
    );

    // Verify all requests reached the server.
    assert_eq!(server.hits().len(), 2);
}
