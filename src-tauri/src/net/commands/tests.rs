//! Tests for the online-service switches and the Discogs token commands.
//! Every request goes to a server on 127.0.0.1 started by the test, and every
//! credential store is the in-memory mock: nothing reaches the internet or
//! the real password store.

use std::cell::Cell;

use super::*;
use crate::net::services::discogs::check_token_at;
use crate::net::tests::{test_db, Probe, Reply, TestDb, TestServer};
use crate::net::Request;

const IDENTITY: &str = include_str!("../services/fixtures/discogs_identity.json");

/// A recognisable stand-in for a real token. If it shows up anywhere it must
/// not, the assertion text shows where.
const SECRET: &str = "TLP-SECRET-TOKEN-9f3a1c7e";

fn identity() -> Reply {
    Reply::Typed(200, "application/json", IDENTITY.as_bytes().to_vec())
}

fn switch_discogs_on(db: &TestDb) {
    set_opt_in(&db.writer, Service::Discogs, true).unwrap();
}

/// Saves `token` the way the command does, asking `server` instead of
/// Discogs.
fn save(
    db: &TestDb,
    credentials: &Credentials,
    server: &TestServer,
    token: &str,
) -> SaveTokenOutcome {
    let net = Net::for_tests(db.reads.clone());
    let base = server.url("");
    save_token(&db.reads, credentials, token, |t| {
        check_token_at(&net, &base, t)
    })
    .unwrap()
}

// --- Switches ---

#[test]
fn every_lookup_service_is_off_on_a_fresh_database() {
    let db = test_db();
    let status = status(&db.reads, &Credentials::in_memory()).unwrap();
    assert!(!status.music_brainz);
    assert!(!status.acoust_id.on);
    assert!(!status.discogs.on);
    assert!(!status.cover_art);
}

#[test]
fn switching_a_service_on_or_off_writes_its_own_opt_in_row_only() {
    let db = test_db();
    let credentials = Credentials::in_memory();
    for service in [
        OnlineService::MusicBrainz,
        OnlineService::AcoustId,
        OnlineService::Discogs,
        OnlineService::CoverArt,
    ] {
        set_service(&db.writer, service, true).unwrap();
        for other in Service::ALL {
            assert_eq!(
                is_opted_in(&db.reads, other).unwrap(),
                other == service.service(),
                "{service:?} on, {other}"
            );
        }
        set_service(&db.writer, service, false).unwrap();
        assert!(!is_opted_in(&db.reads, service.service()).unwrap());
    }
    assert!(!status(&db.reads, &credentials).unwrap().music_brainz);
}

#[test]
fn the_switches_leave_update_checks_and_model_downloads_alone() {
    let db = test_db();
    set_opt_in(&db.writer, Service::UpdateCheck, true).unwrap();
    set_service(&db.writer, OnlineService::AcoustId, true).unwrap();
    set_service(&db.writer, OnlineService::AcoustId, false).unwrap();
    assert!(is_opted_in(&db.reads, Service::UpdateCheck).unwrap());
    assert!(!is_opted_in(&db.reads, Service::ModelDownload).unwrap());
}

#[test]
fn the_status_reports_each_switch() {
    let db = test_db();
    let credentials = Credentials::in_memory();
    set_service(&db.writer, OnlineService::MusicBrainz, true).unwrap();
    set_service(&db.writer, OnlineService::CoverArt, true).unwrap();
    let status = status(&db.reads, &credentials).unwrap();
    assert!(status.music_brainz && status.cover_art);
    assert!(!status.acoust_id.on && !status.discogs.on);
    set_service(&db.writer, OnlineService::AcoustId, true).unwrap();
    assert!(super::status(&db.reads, &credentials).unwrap().acoust_id.on);
}

#[test]
fn the_status_says_when_acoustids_key_is_missing() {
    let db = test_db();
    let status = status(&db.reads, &Credentials::in_memory()).unwrap();
    assert_eq!(status.acoust_id.key_missing, acoustid::key_missing());
}

#[test]
fn the_status_says_whether_a_discogs_token_is_saved_and_whether_there_is_a_store() {
    let db = test_db();
    let credentials = Credentials::in_memory();
    let before = status(&db.reads, &credentials).unwrap().discogs;
    assert!(!before.token_saved && before.store_available);
    credentials.set(credentials::DISCOGS, SECRET).unwrap();
    assert!(status(&db.reads, &credentials).unwrap().discogs.token_saved);
    let none = status(&db.reads, &Credentials::unavailable())
        .unwrap()
        .discogs;
    assert!(!none.token_saved && !none.store_available);
}

#[test]
fn a_store_that_fails_to_answer_counts_as_no_token_in_the_status() {
    let db = test_db();
    let credentials = Credentials::in_memory();
    credentials.set(credentials::DISCOGS, SECRET).unwrap();
    credentials.fail_next_call(credentials::DISCOGS);
    assert!(!status(&db.reads, &credentials).unwrap().discogs.token_saved);
}

// --- Saving the Discogs token ---

#[test]
fn a_token_is_refused_while_the_discogs_switch_is_off_and_nothing_is_asked_or_kept() {
    let db = test_db();
    let credentials = Credentials::in_memory();
    let probe = Probe::new();
    let asked = Cell::new(false);
    let outcome = save_token(&db.reads, &credentials, SECRET, |_| {
        asked.set(true);
        Ok(())
    })
    .unwrap();
    assert_eq!(outcome, SaveTokenOutcome::SwitchOff);
    assert!(!asked.get());
    assert_eq!(probe.connections(), 0);
    assert_eq!(credentials.has(credentials::DISCOGS), Ok(false));
}

#[test]
fn a_token_discogs_accepts_is_kept_in_the_store_and_sent_to_discogs_only_as_a_header() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    let server = TestServer::start(identity());
    let outcome = save(&db, &credentials, &server, &format!("  {SECRET}\n"));
    assert_eq!(outcome, SaveTokenOutcome::Saved);
    assert_eq!(
        credentials.get(credentials::DISCOGS).unwrap().as_deref(),
        Some(SECRET)
    );
    let hits = server.hits();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].request_line.starts_with("GET /oauth/identity"));
    assert_eq!(
        hits[0].header("authorization"),
        Some(format!("Discogs token={SECRET}").as_str())
    );
    assert!(!hits[0].request_line.contains(SECRET));
}

#[test]
fn a_token_discogs_refuses_is_not_kept() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    for status in [401, 403] {
        let server = TestServer::start(Reply::Status(status));
        assert_eq!(
            save(&db, &credentials, &server, SECRET),
            SaveTokenOutcome::Rejected,
            "{status}"
        );
    }
    assert_eq!(credentials.has(credentials::DISCOGS), Ok(false));
}

#[test]
fn a_token_that_could_not_be_checked_is_not_kept() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    for reply in [
        Reply::Status(500),
        Reply::Body(b"not json".to_vec()),
        Reply::Typed(200, "application/json", br#"{"username":""}"#.to_vec()),
    ] {
        let server = TestServer::start(reply);
        assert_eq!(
            save(&db, &credentials, &server, SECRET),
            SaveTokenOutcome::CouldNotCheck
        );
    }
    assert_eq!(credentials.has(credentials::DISCOGS), Ok(false));
}

#[test]
fn an_empty_token_or_one_with_control_characters_asks_nothing() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    let server = TestServer::start(identity());
    for token in ["", "   ", "abc\ndef", "abc\0def"] {
        assert_eq!(
            save(&db, &credentials, &server, token),
            SaveTokenOutcome::Empty,
            "{token:?}"
        );
    }
    assert!(server.hits().is_empty());
    assert_eq!(credentials.has(credentials::DISCOGS), Ok(false));
}

#[test]
fn without_a_password_store_nothing_is_asked_either() {
    let db = test_db();
    switch_discogs_on(&db);
    let server = TestServer::start(identity());
    let outcome = save(&db, &Credentials::unavailable(), &server, SECRET);
    assert_eq!(outcome, SaveTokenOutcome::StoreUnavailable);
    assert!(server.hits().is_empty());
}

#[test]
fn a_store_that_refuses_the_write_is_reported_and_keeps_nothing() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    let server = TestServer::start(identity());
    // The check passes; the store's first call (the write) fails.
    let net = Net::for_tests(db.reads.clone());
    let base = server.url("");
    let outcome = save_token(&db.reads, &credentials, SECRET, |t| {
        credentials.fail_next_call(credentials::DISCOGS);
        check_token_at(&net, &base, t)
    })
    .unwrap();
    assert_eq!(outcome, SaveTokenOutcome::StoreFailed);
    assert_eq!(credentials.has(credentials::DISCOGS), Ok(false));
}

#[test]
fn a_new_token_replaces_the_old_one_only_when_discogs_accepts_it() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    assert_eq!(
        save(
            &db,
            &credentials,
            &TestServer::start(identity()),
            "FAKE-first"
        ),
        SaveTokenOutcome::Saved
    );
    let refusing = TestServer::start(Reply::Status(401));
    assert_eq!(
        save(&db, &credentials, &refusing, "FAKE-second"),
        SaveTokenOutcome::Rejected
    );
    assert_eq!(
        credentials.get(credentials::DISCOGS).unwrap().as_deref(),
        Some("FAKE-first")
    );
    assert_eq!(
        save(
            &db,
            &credentials,
            &TestServer::start(identity()),
            "FAKE-second"
        ),
        SaveTokenOutcome::Saved
    );
    assert_eq!(
        credentials.get(credentials::DISCOGS).unwrap().as_deref(),
        Some("FAKE-second")
    );
}

#[test]
fn a_switch_turned_off_while_the_check_waits_is_reported_as_off() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    let net = Net::for_tests(db.reads.clone());
    let probe = Probe::new();
    let base = crate::net::tests::local_url(probe.addr(), "");
    let outcome = save_token(&db.reads, &credentials, SECRET, |t| {
        set_opt_in(&db.writer, Service::Discogs, false).unwrap();
        check_token_at(&net, &base, t)
    })
    .unwrap();
    assert_eq!(outcome, SaveTokenOutcome::SwitchOff);
    assert_eq!(probe.connections(), 0);
    assert_eq!(credentials.has(credentials::DISCOGS), Ok(false));
}

// --- Token status and removing ---

#[test]
fn the_token_status_follows_saving_and_removing() {
    let credentials = Credentials::in_memory();
    assert_eq!(
        token_status(&credentials),
        TokenStatus {
            saved: false,
            store_available: true
        }
    );
    credentials.set(credentials::DISCOGS, SECRET).unwrap();
    assert!(token_status(&credentials).saved);
    credentials.clear(credentials::DISCOGS).unwrap();
    assert!(!token_status(&credentials).saved);
    // Removing again, with nothing there, is fine.
    credentials.clear(credentials::DISCOGS).unwrap();
    assert_eq!(
        token_status(&Credentials::unavailable()),
        TokenStatus {
            saved: false,
            store_available: false
        }
    );
}

// --- The token goes nowhere else ---

/// Everything the database holds, as text, table by table.
fn database_text(db: &TestDb) -> String {
    db.reads
        .read(|conn| {
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut all = String::new();
            for table in tables {
                let columns: Vec<String> = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                if columns.is_empty() {
                    continue;
                }
                let quoted: Vec<String> =
                    columns.iter().map(|c| format!("quote(\"{c}\")")).collect();
                let sql = format!("SELECT {} FROM \"{table}\"", quoted.join(" || ' ' || "));
                let mut statement = conn.prepare(&sql)?;
                let rows = statement.query_map([], |row| row.get::<_, Option<String>>(0))?;
                for row in rows {
                    all.push_str(&row?.unwrap_or_default());
                    all.push('\n');
                }
            }
            Ok(all)
        })
        .unwrap()
}

#[test]
fn discogs_token_never_appears_in_status_reply() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    let server = TestServer::start(identity());
    let outcome = save(&db, &credentials, &server, SECRET);
    assert_eq!(outcome, SaveTokenOutcome::Saved);

    // Every reply a command can send, as the frontend would receive it, and
    // as a developer would print it.
    let replies = [
        serde_json::to_string(&outcome).unwrap(),
        format!("{outcome:?}"),
        serde_json::to_string(&status(&db.reads, &credentials).unwrap()).unwrap(),
        format!("{:?}", status(&db.reads, &credentials).unwrap()),
        serde_json::to_string(&token_status(&credentials)).unwrap(),
        format!("{:?}", token_status(&credentials)),
        format!("{credentials:?}"),
    ];
    for reply in &replies {
        assert!(!reply.contains(SECRET), "token in a reply: {reply}");
    }
    assert!(replies[2].contains("tokenSaved"), "{}", replies[2]);
}

#[test]
fn discogs_token_never_appears_in_any_error_a_check_can_produce() {
    let db = test_db();
    switch_discogs_on(&db);
    let net = Net::for_tests(db.reads.clone());
    let mut shown = Vec::new();
    for reply in [
        Reply::Status(401),
        Reply::Status(429),
        Reply::Status(500),
        Reply::Body(b"nope".to_vec()),
    ] {
        let server = TestServer::start(reply);
        let error = check_token_at(&net, &server.url(""), SECRET).unwrap_err();
        shown.push(format!("{error} | {error:?}"));
    }
    // Nobody listening, and then the gate's own refusal.
    let error = check_token_at(&net, "http://127.0.0.1:9", SECRET).unwrap_err();
    shown.push(format!("{error} | {error:?}"));
    set_opt_in(&db.writer, Service::Discogs, false).unwrap();
    let error = check_token_at(&net, "http://127.0.0.1:9", SECRET).unwrap_err();
    shown.push(format!("{error} | {error:?}"));
    // The request as it would be logged.
    let request = Request::get(Service::Discogs, "https://api.discogs.com/oauth/identity")
        .header("Authorization", format!("Discogs token={SECRET}"));
    shown.push(format!("{request:?}"));
    for text in shown {
        assert!(!text.contains(SECRET), "token in: {text}");
    }
}

#[test]
fn discogs_token_is_never_written_to_the_database() {
    let db = test_db();
    switch_discogs_on(&db);
    let credentials = Credentials::in_memory();
    let server = TestServer::start(identity());
    assert_eq!(
        save(&db, &credentials, &server, SECRET),
        SaveTokenOutcome::Saved
    );
    let text = database_text(&db);
    assert!(text.contains("discogs"), "the dump found the opt-in row");
    assert!(!text.contains(SECRET));
}

/// The text of every `.rs` file under `dir`, named.
fn sources_under(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        // ci-reads: not a frontend read
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap(); // ci-reads: not a frontend read
            found.push((path.display().to_string(), text));
        }
    }
    found
}

#[test]
fn bundles_and_diagnostics_never_read_the_credential_store() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"); // ci-reads: not a frontend read
    let mut checked = 0;
    for folder in ["bundle", "diagnostics"] {
        for (name, text) in sources_under(&root.join(folder)) {
            checked += 1;
            for word in ["credentials", "Credentials", "keyring", "DISCOGS"] {
                // Prose in the report's own comments may name the word; code
                // may not.
                let code: String = text
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!code.contains(word), "{name} mentions {word}");
            }
        }
    }
    assert!(checked >= 6, "the scan found only {checked} files");
}

#[test]
fn the_credential_code_prints_and_logs_nothing() {
    for (name, text) in [
        ("credentials.rs", include_str!("../../credentials.rs")),
        ("net/commands.rs", include_str!("../commands.rs")),
    ] {
        for word in ["println!", "eprintln!", "dbg!", "log::", "tracing::"] {
            assert!(!text.contains(word), "{name} uses {word}");
        }
    }
}
