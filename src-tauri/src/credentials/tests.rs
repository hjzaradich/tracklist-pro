//! Tests for the password-store wrapper. Every test uses the in-memory mock
//! store; none touches the operating system's real one.

use super::*;

const TOKEN: &str = "FAKE-token-for-tests-1111";

#[test]
fn a_secret_set_can_be_read_back_and_is_reported_as_kept() {
    let store = Credentials::in_memory();
    assert_eq!(store.has(DISCOGS), Ok(false));
    store.set(DISCOGS, TOKEN).unwrap();
    assert_eq!(store.has(DISCOGS), Ok(true));
    assert_eq!(store.get(DISCOGS).unwrap().as_deref(), Some(TOKEN));
}

#[test]
fn setting_again_replaces_the_secret() {
    let store = Credentials::in_memory();
    store.set(DISCOGS, TOKEN).unwrap();
    store.set(DISCOGS, "FAKE-second-token").unwrap();
    assert_eq!(
        store.get(DISCOGS).unwrap().as_deref(),
        Some("FAKE-second-token")
    );
}

#[test]
fn clearing_forgets_the_secret_and_clearing_a_missing_one_is_fine() {
    let store = Credentials::in_memory();
    store.clear(DISCOGS).unwrap();
    store.set(DISCOGS, TOKEN).unwrap();
    store.clear(DISCOGS).unwrap();
    assert_eq!(store.has(DISCOGS), Ok(false));
    assert_eq!(store.get(DISCOGS), Ok(None));
    store.clear(DISCOGS).unwrap();
}

#[test]
fn each_provider_has_its_own_entry() {
    let store = Credentials::in_memory();
    store.set(DISCOGS, TOKEN).unwrap();
    assert_eq!(store.has("another"), Ok(false));
    store.set("another", "FAKE-other").unwrap();
    store.clear(DISCOGS).unwrap();
    assert_eq!(store.get("another").unwrap().as_deref(), Some("FAKE-other"));
}

#[test]
fn a_machine_without_a_store_says_unavailable_and_keeps_nothing() {
    let store = Credentials::unavailable();
    assert!(!store.is_available());
    assert_eq!(store.set(DISCOGS, TOKEN), Err(CredentialError::Unavailable));
    assert_eq!(store.get(DISCOGS), Err(CredentialError::Unavailable));
    assert_eq!(store.has(DISCOGS), Err(CredentialError::Unavailable));
    assert_eq!(store.clear(DISCOGS), Err(CredentialError::Unavailable));
}

#[test]
fn a_store_that_fails_says_failed_and_works_again_afterwards() {
    let store = Credentials::in_memory();
    store.fail_next_call(DISCOGS);
    assert_eq!(store.set(DISCOGS, TOKEN), Err(CredentialError::Failed));
    store.set(DISCOGS, TOKEN).unwrap();
    store.fail_next_call(DISCOGS);
    assert_eq!(store.has(DISCOGS), Err(CredentialError::Failed));
    store.fail_next_call(DISCOGS);
    assert_eq!(store.clear(DISCOGS), Err(CredentialError::Failed));
}

#[test]
fn the_store_service_name_is_this_apps_own_and_not_musicmanagers() {
    assert_eq!(SERVICE_NAME, "com.tracklistpro.desktop");
    assert_ne!(SERVICE_NAME, "com.musicmanager.desktop");
    assert_eq!(DISCOGS, "discogs");
}

#[test]
fn entries_are_filed_under_the_service_name_and_provider() {
    let mock = keyring_core::mock::Store::new().unwrap();
    let store = Credentials::with_store(mock.clone());
    store.set(DISCOGS, TOKEN).unwrap();
    let kept: Vec<(String, String)> = mock
        .inner
        .lock()
        .unwrap()
        .borrow()
        .iter()
        .map(|cred| (cred.specifiers.0.clone(), cred.specifiers.1.clone()))
        .collect();
    assert_eq!(
        kept,
        [("com.tracklistpro.desktop".to_owned(), "discogs".to_owned())]
    );
}

#[test]
fn a_test_build_never_reaches_the_real_store() {
    // `native()` is the in-memory store while tests run, so even a test that
    // starts the whole app can't read or write the user's real credentials.
    let a = Credentials::native();
    assert!(a.is_available());
    a.set(DISCOGS, TOKEN).unwrap();
    // A second call makes a store of its own: nothing is shared with a
    // lasting store.
    let b = Credentials::native();
    assert_eq!(b.has(DISCOGS), Ok(false));
}

#[test]
fn debug_output_shows_whether_there_is_a_store_and_never_a_secret() {
    let store = Credentials::in_memory();
    store.set(DISCOGS, TOKEN).unwrap();
    let shown = format!("{store:?} {:?}", CredentialError::Failed);
    assert!(!shown.contains(TOKEN), "{shown}");
    assert!(shown.contains("available"));
}

#[test]
fn errors_say_nothing_about_the_secret_or_the_store() {
    for e in [CredentialError::Unavailable, CredentialError::Failed] {
        let text = e.to_string();
        assert!(
            !text.contains(TOKEN) && !text.contains(SERVICE_NAME),
            "{text}"
        );
    }
}
