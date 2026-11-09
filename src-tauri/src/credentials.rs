//! The user's own secrets (ROADMAP 2.2): today only the Discogs token, kept
//! in the operating system's password store (Windows Credential Manager, the
//! macOS keychain). Linux has no store here: every call says "unavailable".
//!
//! A secret lives in the store and nowhere else. It is never in the database
//! (a token there would travel with the portability bundle), the bundle, the
//! diagnostics report, the event log, an error message or an IPC reply. The
//! only way to read one is [`Credentials::get`], which is crate-internal and
//! meant for the code that sends it to the service it belongs to.
//!
//! The store is injected: the app builds [`Credentials::native`] once at
//! start; every test uses [`Credentials::with_store`] and the mock store, so
//! no test touches a real one.

use std::sync::Arc;

use keyring_core::{CredentialStore, Error};

/// The store's service name. Distinct from musicmanager's
/// `com.musicmanager.desktop`, so the two apps never share entries.
pub const SERVICE_NAME: &str = "com.tracklistpro.desktop";

/// The entry name of the user's Discogs token.
pub const DISCOGS: &str = "discogs";

/// Why a store call failed. Says nothing about the secret or the store's own
/// message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    /// This platform has no password store, or it couldn't be opened.
    Unavailable,
    /// The store refused or failed (locked, denied, damaged).
    Failed,
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CredentialError::Unavailable => "there is no password store",
            CredentialError::Failed => "the password store failed",
        })
    }
}

impl std::error::Error for CredentialError {}

/// The password store, or the lack of one. Cheap to clone.
#[derive(Clone)]
pub struct Credentials {
    store: Option<Arc<CredentialStore>>,
}

/// Shows whether there is a store, never an entry.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("available", &self.store.is_some())
            .finish()
    }
}

impl Credentials {
    /// This platform's own store. Only the app's start calls it. In a test
    /// build it gives an in-memory store instead, so even a test that runs
    /// the app's start can't reach a real one. Unit tests are `cfg(test)`;
    /// integration tests (tests/*.rs) get the same through the `test-store`
    /// feature, which only the dev-dependency on this crate turns on.
    pub fn native() -> Credentials {
        if cfg!(any(test, feature = "test-store")) {
            return Credentials::in_memory();
        }
        #[cfg(windows)]
        let store: Result<Arc<CredentialStore>, Error> =
            windows_native_keyring_store::Store::new().map(|s| s as Arc<CredentialStore>);
        #[cfg(target_os = "macos")]
        let store: Result<Arc<CredentialStore>, Error> =
            apple_native_keyring_store::keychain::Store::new().map(|s| s as Arc<CredentialStore>);
        #[cfg(not(any(windows, target_os = "macos")))]
        let store: Result<Arc<CredentialStore>, Error> = Err(Error::NoDefaultStore);
        Credentials { store: store.ok() }
    }

    /// Uses `store`, e.g. the mock store in tests.
    pub fn with_store(store: Arc<CredentialStore>) -> Credentials {
        Credentials { store: Some(store) }
    }

    /// A store that lives in memory and is gone with the process: the mock
    /// store of keyring-core. For tests.
    pub fn in_memory() -> Credentials {
        match keyring_core::mock::Store::new() {
            Ok(store) => Credentials::with_store(store),
            Err(_) => Credentials::unavailable(),
        }
    }

    /// No store at all: what Linux gets.
    pub fn unavailable() -> Credentials {
        Credentials { store: None }
    }

    /// Whether secrets can be kept on this machine.
    pub fn is_available(&self) -> bool {
        self.store.is_some()
    }

    fn entry(&self, provider: &str) -> Result<keyring_core::Entry, CredentialError> {
        let store = self.store.as_ref().ok_or(CredentialError::Unavailable)?;
        store
            .build(SERVICE_NAME, provider, None)
            .map_err(|_| CredentialError::Failed)
    }

    /// Keeps `secret` for `provider`, replacing any earlier one.
    pub fn set(&self, provider: &str, secret: &str) -> Result<(), CredentialError> {
        self.entry(provider)?
            .set_password(secret)
            .map_err(|_| CredentialError::Failed)
    }

    /// The secret kept for `provider`. Crate-internal: no command returns it.
    // Used by the look-up (ROADMAP 2.2) to sign Discogs requests.
    pub(crate) fn get(&self, provider: &str) -> Result<Option<String>, CredentialError> {
        match self.entry(provider)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialError::Failed),
        }
    }

    /// Forgets the secret kept for `provider`. Forgetting a missing one is
    /// fine.
    pub fn clear(&self, provider: &str) -> Result<(), CredentialError> {
        match self.entry(provider)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => Err(CredentialError::Failed),
        }
    }

    /// Makes the next call on `provider`'s entry fail, as a locked or
    /// denied store would. In-memory stores only.
    #[cfg(test)]
    pub(crate) fn fail_next_call(&self, provider: &str) {
        let entry = self.entry(provider).unwrap();
        let cred: &keyring_core::mock::Cred = entry.as_any().downcast_ref().unwrap();
        cred.set_error(Error::Invalid("test".into(), "the store refused".into()));
    }

    /// Whether a secret is kept for `provider`.
    pub fn has(&self, provider: &str) -> Result<bool, CredentialError> {
        match self.entry(provider)?.get_password() {
            Ok(_) => Ok(true),
            Err(Error::NoEntry) => Ok(false),
            Err(_) => Err(CredentialError::Failed),
        }
    }
}

#[cfg(test)]
mod tests;
