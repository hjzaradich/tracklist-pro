//! Integration tests get the mock password store, never a real one.
//!
//! Unit tests are `cfg(test)`, which makes `Credentials::native()` the
//! in-memory mock store. Integration tests (tests/*.rs) compile the library
//! without `cfg(test)`; the `test-store` feature, turned on by the
//! dev-dependency on this crate in Cargo.toml, gives them the same.

use tracklist_pro_lib::credentials::{self, Credentials};

#[test]
fn an_integration_test_that_asks_for_the_native_store_gets_the_in_memory_mock() {
    let first = Credentials::native();
    assert!(first.is_available());
    first.set(credentials::DISCOGS, "FAKE-token").unwrap();
    assert_eq!(first.has(credentials::DISCOGS), Ok(true));
    // A real, lasting store would show the token to a second handle. Each
    // mock store is its own, so this one sees nothing.
    let second = Credentials::native();
    assert_eq!(second.has(credentials::DISCOGS), Ok(false));
}
