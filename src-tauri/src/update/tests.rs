//! Update checks: the stored answers, the once-a-day rule and what the
//! status says. The request itself is tested with a local server in
//! `net::services::update`; here the fetch is a closure, so nothing
//! connects.

use std::cell::Cell;

use super::*;
use crate::ipc::testing::{app, invoke};
use serde_json::json;

fn db() -> (tempfile::TempDir, Writer, ReadPool) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(dir.path(), "u.db")).unwrap();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    (dir, writer, reads)
}

fn release(version: &str) -> Release {
    Release {
        version: version.to_owned(),
        url: format!("https://github.com/hjzaradich/tracklist-pro/releases/tag/v{version}"),
    }
}

/// A fetch that counts its calls.
fn fetch_of<'a>(
    calls: &'a Cell<u32>,
    answer: Result<Option<Release>, ClientError>,
) -> impl FnOnce() -> Result<Option<Release>, ClientError> + 'a {
    move || {
        calls.set(calls.get() + 1);
        answer
    }
}

fn age_the_answer(writer: &Writer, modifier: &str) {
    let sql = format!(
        "UPDATE setting SET value = json_set(value, '$.checkedAt', \
         strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '{modifier}')) WHERE key = 'update_check_result'"
    );
    writer.call(move |c| c.execute(&sql, [])).unwrap();
}

#[test]
fn a_fresh_database_is_off_and_unasked() {
    let (_dir, _writer, reads) = db();
    let status = reads.read(|c| status_for(c, "0.1.9")).unwrap();
    assert!(!status.enabled && !status.asked);
    assert_eq!(status.available, None);
    assert_eq!(status.checked_at, None);
    assert_eq!(status.current, "0.1.9");
}

#[test]
fn with_update_checks_off_nothing_is_asked_even_when_forced() {
    let (_dir, writer, reads) = db();
    let calls = Cell::new(0);
    for force in [false, true] {
        let status = check_with(
            &reads,
            &writer,
            force,
            fetch_of(&calls, Ok(Some(release("9.0.0")))),
        )
        .unwrap();
        assert_eq!(status.available, None);
    }
    assert_eq!(calls.get(), 0);
}

#[test]
fn a_newer_release_is_found_stored_and_reported() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    let calls = Cell::new(0);
    let status = check_with(
        &reads,
        &writer,
        false,
        fetch_of(&calls, Ok(Some(release("9.0.0")))),
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert!(status.enabled);
    assert_eq!(
        status.available,
        Some(Available {
            version: "9.0.0".into(),
            url: release("9.0.0").url,
        })
    );
    assert!(status.checked_at.is_some());
}

#[test]
fn the_same_or_an_older_release_is_nothing_newer() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    for version in [CURRENT, "0.0.1"] {
        let calls = Cell::new(0);
        let status = check_with(
            &reads,
            &writer,
            true,
            fetch_of(&calls, Ok(Some(release(version)))),
        )
        .unwrap();
        assert_eq!(status.available, None, "{version}");
        assert!(status.checked_at.is_some());
    }
}

#[test]
fn no_release_yet_is_stored_as_an_answer_and_is_nothing_newer() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    let calls = Cell::new(0);
    let status = check_with(&reads, &writer, false, fetch_of(&calls, Ok(None))).unwrap();
    assert_eq!(status.available, None);
    assert!(status.checked_at.is_some());
}

#[test]
fn startup_checks_at_most_once_a_day_but_check_now_always_asks() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    let calls = Cell::new(0);
    let ask = |force: bool| {
        check_with(&reads, &writer, force, fetch_of(&calls, Ok(None))).unwrap();
    };
    ask(false);
    assert_eq!(calls.get(), 1);
    // Just checked: another start does not ask.
    ask(false);
    assert_eq!(calls.get(), 1);
    // Check now asks regardless.
    ask(true);
    assert_eq!(calls.get(), 2);
    // Twenty-three hours later: still not due.
    age_the_answer(&writer, "-23 hours");
    ask(false);
    assert_eq!(calls.get(), 2);
    // Over a day later: due.
    age_the_answer(&writer, "-25 hours");
    ask(false);
    assert_eq!(calls.get(), 3);
}

#[test]
fn a_failed_check_changes_nothing_and_is_tried_again_at_the_next_start() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    let calls = Cell::new(0);
    let failure = || {
        Err(ClientError::Status {
            service: Service::UpdateCheck,
            status: 500,
        })
    };
    let status = check_with(&reads, &writer, false, fetch_of(&calls, failure())).unwrap();
    assert_eq!((status.available, status.checked_at), (None, None));
    check_with(&reads, &writer, false, fetch_of(&calls, failure())).unwrap();
    assert_eq!(calls.get(), 2);
}

#[test]
fn a_failed_check_keeps_the_earlier_answer() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    let calls = Cell::new(0);
    check_with(
        &reads,
        &writer,
        true,
        fetch_of(&calls, Ok(Some(release("9.0.0")))),
    )
    .unwrap();
    let failure = Err(ClientError::NoId);
    let status = check_with(&reads, &writer, true, fetch_of(&calls, failure)).unwrap();
    assert_eq!(status.available.map(|a| a.version), Some("9.0.0".into()));
}

#[test]
fn a_stored_answer_goes_stale_once_the_app_is_that_version() {
    let (_dir, writer, reads) = db();
    set_opt_in(&writer, Service::UpdateCheck, true).unwrap();
    let calls = Cell::new(0);
    check_with(
        &reads,
        &writer,
        true,
        fetch_of(&calls, Ok(Some(release("0.5.0")))),
    )
    .unwrap();
    let before = reads.read(|c| status_for(c, "0.4.0")).unwrap();
    let after = reads.read(|c| status_for(c, "0.5.0")).unwrap();
    assert!(before.available.is_some());
    assert_eq!(after.available, None);
}

#[test]
fn an_unreadable_stored_answer_is_no_answer() {
    let (_dir, writer, reads) = db();
    writer
        .call(|c| write_setting(c, RESULT, "{\"surprise\":1}"))
        .unwrap();
    let status = reads.read(|c| status_for(c, "0.1.0")).unwrap();
    assert_eq!((status.available, status.checked_at), (None, None));
    assert!(reads.read(due).unwrap());
}

#[test]
fn answering_no_is_remembered_and_leaves_checks_off() {
    let (_data, app) = app();
    assert_eq!(
        invoke(&app, "update_status", json!({})).unwrap()["asked"],
        json!(false)
    );
    invoke(&app, "set_update_checks", json!({"on": false})).unwrap();
    let status = invoke(&app, "update_status", json!({})).unwrap();
    assert_eq!(status["asked"], json!(true));
    assert_eq!(status["enabled"], json!(false));
}

#[test]
fn answering_yes_turns_the_service_on_and_the_switch_can_turn_it_off_again() {
    let (_data, app) = app();
    invoke(&app, "set_update_checks", json!({"on": true})).unwrap();
    let status = invoke(&app, "update_status", json!({})).unwrap();
    assert_eq!(
        (status["asked"].clone(), status["enabled"].clone()),
        (json!(true), json!(true))
    );
    invoke(&app, "set_update_checks", json!({"on": false})).unwrap();
    let status = invoke(&app, "update_status", json!({})).unwrap();
    assert_eq!(
        (status["asked"].clone(), status["enabled"].clone()),
        (json!(true), json!(false))
    );
}

#[test]
fn check_now_while_off_sends_nothing_and_is_not_an_error() {
    let (_data, app) = app();
    let status = invoke(&app, "check_for_updates", json!({"force": true})).unwrap();
    assert_eq!(status["enabled"], json!(false));
    assert_eq!(status["checkedAt"], json!(null));
}
