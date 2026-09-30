//! The one error type every command returns (1aA-12).
//!
//! A command that can fail returns `Result<T, IpcError>`. The frontend gets
//! the error's [`ErrorKind`] and its parameters, never error text: raw
//! database or OS messages are English, technical, and can name tables or
//! paths. The frontend turns the kind into words from the locale files,
//! through the generated `ERROR_KEYS` table (`src/api/errors.ts`).
//!
//! The detail behind an error goes to stderr for developers, not to the UI.
//!
//! To add a kind, add one entry to the list below: its doc comment, its
//! name, its i18n key and the names of its parameters. General errors keep
//! their message in the `errors` namespace; a feature's own errors can keep
//! theirs in the feature's namespace (e.g. `musicFolders:insideMusicFolder`).
//! Then give it a message in that locale file, using exactly those
//! parameters; the tests fail until they agree. Build it with
//! [`IpcError::new`], or [`IpcError::with`] if it takes parameters, usually
//! in a `From<YourError> for IpcError`.

use std::collections::BTreeMap;
use std::fmt;

use rusqlite::ErrorCode;
use serde::Serialize;
use specta::Type;

use crate::db::DbError;
use crate::ops::OpsError;

/// Declares [`ErrorKind`] with its `ALL` list, keys and parameters from one
/// entry per kind, so no kind can be left out of any of them.
macro_rules! error_kinds {
    ($(
        $(#[doc = $doc:literal])*
        $kind:ident => $key:literal, params: [$($param:literal),*];
    )*) => {
        /// What went wrong, in terms the user can act on. Each kind has one
        /// message in the locale files.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Type)]
        #[serde(rename_all = "camelCase")]
        pub enum ErrorKind {
            $($(#[doc = $doc])* $kind,)*
        }

        impl ErrorKind {
            /// Every kind, in declaration order.
            pub const ALL: &'static [ErrorKind] = &[$(ErrorKind::$kind),*];

            /// The i18n key of this kind's message, with its namespace.
            pub fn key(self) -> &'static str {
                match self {
                    $(ErrorKind::$kind => $key,)*
                }
            }

            /// The parameters this kind's message is filled with. An error
            /// of this kind carries exactly these, and its message uses no
            /// others.
            pub fn params(self) -> &'static [&'static str] {
                match self {
                    $(ErrorKind::$kind => &[$($param),*],)*
                }
            }
        }
    };
}

// One entry per kind, each kept together, so lanes adding kinds don't
// touch each other's lines.
error_kinds! {
    /// The database was busy with another write for too long.
    Busy => "errors:busy", params: [];
    /// The disk holding the app data folder is full.
    DiskFull => "errors:diskFull", params: [];
    /// The app data folder can't be written (read-only, or no permission).
    CannotWrite => "errors:cannotWrite", params: [];
    /// The database file is damaged, or isn't a database.
    Damaged => "errors:damaged", params: [];
    /// Any other database failure.
    Database => "errors:database", params: [];
    /// The database writer has stopped; only a restart brings it back.
    Stopped => "errors:stopped", params: [];
    /// A bug or an impossible state: the app did something it shouldn't.
    Internal => "errors:internal", params: [];
    /// The path given as a music folder isn't a folder that exists.
    NotAFolder => "musicFolders:notAFolder", params: ["path"];
    /// The path can't be stored as a music folder: not a full path from a
    /// drive or share, a name Windows doesn't allow, or an unreadable drive.
    BadPath => "musicFolders:badPath", params: ["path"];
    /// That folder is already a music folder.
    AlreadyAdded => "musicFolders:alreadyAdded", params: ["path"];
    /// The folder is inside the existing music folder `musicFolder`.
    InsideMusicFolder => "musicFolders:insideMusicFolder", params: ["musicFolder"];
    /// The folder contains the existing music folder `musicFolder`.
    ContainsMusicFolder => "musicFolders:containsMusicFolder", params: ["musicFolder"];
    /// There's no music folder with that id.
    MusicFolderNotFound => "musicFolders:notFound", params: [];
    /// Tracks, Library tracks or relinks still use files in the music
    /// folder, so it can't be removed.
    MusicFolderInUse => "musicFolders:inUse", params: [];
    /// The rekordbox export isn't a file that exists.
    RekordboxXmlNotFound => "rekordbox:error.notFound", params: ["path"];
    /// The file isn't a rekordbox collection export.
    NotRekordboxXml => "rekordbox:error.notAnExport", params: ["path"];
    /// No rekordbox export has been chosen yet.
    NoRekordboxXml => "rekordbox:error.noneChosen", params: [];
}

/// Parameter names i18next reads as options, not as values to fill in: a
/// parameter with one of these names would change which message is shown or
/// how. `count` picks a plural form, which the key lookup doesn't handle yet
/// (see the tests), so it's refused too until a message needs plurals.
pub const RESERVED_PARAMS: &[&str] = &[
    "count",
    "context",
    "defaultValue",
    "fallbackLng",
    "interpolation",
    "joinArrays",
    "keySeparator",
    "lng",
    "lngs",
    "ns",
    "nsSeparator",
    "ordinal",
    "postProcess",
    "replace",
    "returnDetails",
    "returnObjects",
    "skipInterpolation",
];

/// Every kind's i18n key, exported to the bindings as `ERROR_KEYS` so the
/// frontend looks messages up with keys TypeScript can check.
pub fn error_keys() -> BTreeMap<ErrorKind, &'static str> {
    ErrorKind::ALL.iter().map(|&k| (k, k.key())).collect()
}

/// A value filled into an error message: data (a name, a path, a count),
/// never prose. Numbers stay numbers so the frontend formats them. They're
/// 32-bit whole numbers, so every one is exact in a TypeScript `number` and
/// there's no NaN to send; add a finite decimal when a message needs one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(untagged)]
pub enum ErrorParam {
    Text(String),
    Number(i32),
}

impl From<String> for ErrorParam {
    fn from(text: String) -> Self {
        ErrorParam::Text(text)
    }
}

impl From<&str> for ErrorParam {
    fn from(text: &str) -> Self {
        ErrorParam::Text(text.to_owned())
    }
}

impl From<i32> for ErrorParam {
    fn from(n: i32) -> Self {
        ErrorParam::Number(n)
    }
}

/// Why a command failed, as the frontend receives it: a kind and the
/// parameters its message needs. Never error text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    kind: ErrorKind,
    params: BTreeMap<String, ErrorParam>,
}

impl IpcError {
    /// An error of `kind`. Panics in debug builds if the kind's message
    /// needs parameters: use [`IpcError::with`] for those.
    pub fn new(kind: ErrorKind) -> IpcError {
        debug_assert!(kind.params().is_empty(), "{kind:?} needs parameters");
        IpcError {
            kind,
            params: BTreeMap::new(),
        }
    }

    /// An error of `kind` with its message's parameters, e.g.
    /// `IpcError::with(kind, [("path", path.into())])`. Panics in debug
    /// builds unless they're exactly the ones [`ErrorKind::params`] names.
    pub fn with<'a>(
        kind: ErrorKind,
        params: impl IntoIterator<Item = (&'a str, ErrorParam)>,
    ) -> IpcError {
        let params: BTreeMap<String, ErrorParam> = params
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        debug_assert!(
            !params
                .keys()
                .any(|name| RESERVED_PARAMS.contains(&name.as_str())),
            "{kind:?}: a parameter name is an i18next option"
        );
        debug_assert!(
            params.keys().map(String::as_str).eq(sorted(kind.params())),
            "{kind:?} takes {:?}, got {:?}",
            kind.params(),
            params.keys().collect::<Vec<_>>()
        );
        IpcError { kind, params }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn params(&self) -> &BTreeMap<String, ErrorParam> {
        &self.params
    }

    /// Logs `detail` for developers and returns an error of `kind`.
    fn logged(kind: ErrorKind, detail: &dyn fmt::Display) -> IpcError {
        eprintln!("command failed ({kind:?}): {detail}");
        IpcError::new(kind)
    }
}

fn sorted(names: &[&'static str]) -> Vec<&'static str> {
    let mut names = names.to_vec();
    names.sort_unstable();
    names
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind.key())
    }
}

impl std::error::Error for IpcError {}

/// The kind a database error is shown as.
fn db_kind(e: &DbError) -> ErrorKind {
    match e {
        // A damaged file system under the database file.
        DbError::Sqlite(rusqlite::Error::SqliteFailure(f, _))
            if f.extended_code == rusqlite::ffi::SQLITE_IOERR_CORRUPTFS =>
        {
            ErrorKind::Damaged
        }
        DbError::Sqlite(e) => match e.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy) => ErrorKind::Busy,
            // Without shared cache, LOCKED is a conflict inside one
            // connection: a bug, not another writer.
            Some(ErrorCode::DatabaseLocked) => ErrorKind::Internal,
            Some(ErrorCode::DiskFull) => ErrorKind::DiskFull,
            Some(ErrorCode::ReadOnly | ErrorCode::PermissionDenied | ErrorCode::CannotOpen) => {
                ErrorKind::CannotWrite
            }
            Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => ErrorKind::Damaged,
            _ => ErrorKind::Database,
        },
        DbError::WriterGone => ErrorKind::Stopped,
        DbError::Spawn(_)
        | DbError::JobPanicked
        | DbError::Reentrant
        | DbError::Migration(_)
        | DbError::NotWal(_)
        | DbError::NestedRead => ErrorKind::Internal,
    }
}

impl From<DbError> for IpcError {
    fn from(e: DbError) -> Self {
        IpcError::logged(db_kind(&e), &e)
    }
}

impl From<OpsError> for IpcError {
    fn from(e: OpsError) -> Self {
        let kind = match &e {
            OpsError::Db(db) => db_kind(db),
            // A caller's mistake, or a log edited by hand.
            OpsError::BadKind(_)
            | OpsError::DetailsNotObject
            | OpsError::NotRecordable(_)
            | OpsError::NotAField { .. }
            | OpsError::RowNotFound { .. }
            | OpsError::BadStoredValue { .. }
            | OpsError::BadAction(_)
            | OpsError::Referenced { .. }
            | OpsError::NotFinite(_) => ErrorKind::Internal,
        };
        IpcError::logged(kind, &e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::testing::{app, invoke};
    use serde_json::{json, Value};
    use tauri::Manager;

    /// An English locale file (a namespace), as the frontend loads it.
    fn namespace(ns: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../src/locales/en")
            .join(format!("{ns}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("no locale file {}: {e}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    /// The English message for an i18n key such as `errors:busy` or
    /// `musicFolders:error.inside`. Plural keys (`_one`, `_other`) will need
    /// this lookup extended, and `count` taken off [`RESERVED_PARAMS`].
    fn message(key: &str) -> Option<String> {
        let (ns, path) = key.split_once(':')?;
        let mut value = namespace(ns);
        for part in path.split('.') {
            value = value.get(part)?.clone();
        }
        value.as_str().map(str::to_owned)
    }

    /// The `{{name}}` placeholders in a message.
    fn placeholders(message: &str) -> Vec<&str> {
        let mut found: Vec<&str> = message
            .split("{{")
            .skip(1)
            .filter_map(|rest| rest.split("}}").next())
            .map(|p| p.split(',').next().unwrap().trim())
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    fn sqlite(code: std::ffi::c_int) -> DbError {
        DbError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            Some("no such table: secret_table_name".into()),
        ))
    }

    #[test]
    fn every_error_kind_has_an_english_message_under_its_key() {
        for &kind in ErrorKind::ALL {
            let message = message(kind.key())
                .unwrap_or_else(|| panic!("no message `{}` for {kind:?}", kind.key()));
            assert!(!message.trim().is_empty(), "{kind:?}'s message is empty");
        }
    }

    #[test]
    fn every_message_in_the_errors_namespace_belongs_to_an_error_kind() {
        let keys: Vec<&str> = ErrorKind::ALL.iter().map(|k| k.key()).collect();
        let Value::Object(messages) = namespace("errors") else {
            panic!("errors.json is not an object");
        };
        for name in messages.keys() {
            let key = format!("errors:{name}");
            assert!(keys.contains(&key.as_str()), "no error kind uses `{key}`");
        }
    }

    #[test]
    fn each_message_uses_exactly_the_parameters_its_kind_carries() {
        for &kind in ErrorKind::ALL {
            let message = message(kind.key()).unwrap();
            assert_eq!(
                placeholders(&message),
                sorted(kind.params()),
                "{kind:?}: {message}"
            );
        }
    }

    #[test]
    fn no_error_kind_takes_a_parameter_i18next_would_read_as_an_option() {
        for &kind in ErrorKind::ALL {
            for param in kind.params() {
                assert!(
                    !RESERVED_PARAMS.contains(param),
                    "{kind:?}'s parameter `{param}` is an i18next option"
                );
            }
        }
    }

    #[test]
    fn each_error_kind_has_its_own_namespaced_key() {
        let mut keys: Vec<&str> = ErrorKind::ALL.iter().map(|k| k.key()).collect();
        for key in &keys {
            let (ns, path) = key.split_once(':').unwrap_or_else(|| panic!("{key}"));
            assert!(!ns.is_empty() && !path.is_empty(), "{key}");
        }
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ErrorKind::ALL.len(), "two kinds share a key");
    }

    /// The macro with a kind that takes parameters and keeps its message in
    /// a feature's namespace. It's declared in a module of its own so it
    /// stays out of the real list; real kinds, the app's and the features',
    /// are all entries in the one list at the top of this file.
    mod feature_kind {
        use super::super::*;

        error_kinds! {
            /// A made-up kind.
            InsideMusicFolder => "musicFolders:insideMusicFolder", params: ["path", "musicFolder"];
        }

        #[test]
        fn a_kind_declares_its_key_and_parameters_in_one_entry() {
            assert_eq!(ErrorKind::ALL, [ErrorKind::InsideMusicFolder]);
            assert_eq!(
                ErrorKind::InsideMusicFolder.key(),
                "musicFolders:insideMusicFolder"
            );
            assert_eq!(
                ErrorKind::InsideMusicFolder.params(),
                ["path", "musicFolder"]
            );
            assert_eq!(
                serde_json::to_value(ErrorKind::InsideMusicFolder).unwrap(),
                "insideMusicFolder"
            );
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "takes")]
    fn an_error_given_parameters_its_kind_does_not_take_is_caught_in_debug_builds() {
        IpcError::with(ErrorKind::Internal, [("path", "C:\\Music".into())]);
    }

    #[test]
    fn database_errors_are_shown_by_what_the_user_can_do_about_them() {
        use rusqlite::ffi;
        for (error, kind) in [
            (sqlite(ffi::SQLITE_BUSY), ErrorKind::Busy),
            (sqlite(ffi::SQLITE_BUSY_SNAPSHOT), ErrorKind::Busy),
            (sqlite(ffi::SQLITE_LOCKED), ErrorKind::Internal),
            (sqlite(ffi::SQLITE_FULL), ErrorKind::DiskFull),
            (sqlite(ffi::SQLITE_READONLY), ErrorKind::CannotWrite),
            (sqlite(ffi::SQLITE_PERM), ErrorKind::CannotWrite),
            (sqlite(ffi::SQLITE_CANTOPEN), ErrorKind::CannotWrite),
            (sqlite(ffi::SQLITE_CORRUPT), ErrorKind::Damaged),
            (sqlite(ffi::SQLITE_NOTADB), ErrorKind::Damaged),
            (sqlite(ffi::SQLITE_IOERR_CORRUPTFS), ErrorKind::Damaged),
            (sqlite(ffi::SQLITE_IOERR_WRITE), ErrorKind::Database),
            (sqlite(ffi::SQLITE_ERROR), ErrorKind::Database),
            (
                DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows),
                ErrorKind::Database,
            ),
            (DbError::WriterGone, ErrorKind::Stopped),
            (DbError::JobPanicked, ErrorKind::Internal),
            (DbError::Reentrant, ErrorKind::Internal),
            (DbError::NestedRead, ErrorKind::Internal),
            (DbError::NotWal("delete".into()), ErrorKind::Internal),
            (
                DbError::Spawn(std::io::Error::other("no threads")),
                ErrorKind::Internal,
            ),
        ] {
            let shown = format!("{error:?}");
            assert_eq!(IpcError::from(error).kind(), kind, "{shown}");
        }
    }

    #[test]
    fn operation_log_errors_are_shown_as_their_database_error_or_as_internal() {
        assert_eq!(
            IpcError::from(OpsError::Db(sqlite(rusqlite::ffi::SQLITE_FULL))).kind(),
            ErrorKind::DiskFull
        );
        for error in [
            OpsError::BadKind("Edit Fields".into()),
            OpsError::DetailsNotObject,
            OpsError::NotRecordable("change".into()),
            OpsError::NotAField {
                entity: "crate".into(),
                field: "colour".into(),
            },
            OpsError::RowNotFound {
                entity: "crate".into(),
                id: 7,
            },
            OpsError::BadStoredValue {
                field: "bpm".into(),
                value: "fast".into(),
            },
            OpsError::BadAction("rename".into()),
            OpsError::Referenced {
                entity: "crate".into(),
                id: 7,
                by: "crate_track".into(),
            },
            OpsError::NotFinite("bpm".into()),
        ] {
            assert_eq!(IpcError::from(error).kind(), ErrorKind::Internal);
        }
    }

    #[test]
    fn an_error_reaches_the_frontend_as_its_kind_and_params_without_error_text() {
        let error = IpcError::from(sqlite(rusqlite::ffi::SQLITE_ERROR));
        let sent = serde_json::to_value(&error).unwrap();
        assert_eq!(sent, json!({ "kind": "database", "params": {} }));
        assert!(!sent.to_string().contains("secret_table_name"), "{sent}");
    }

    #[test]
    fn error_params_cross_as_plain_strings_and_numbers() {
        let error = IpcError {
            kind: ErrorKind::Internal,
            params: BTreeMap::from([
                ("name".to_owned(), "Peak".into()),
                ("files".to_owned(), 3.into()),
            ]),
        };
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({ "kind": "internal", "params": { "files": 3, "name": "Peak" } })
        );
    }

    // Through the real app and its command list, called the way the
    // frontend calls them.

    #[test]
    fn a_failing_key_notation_command_sends_an_error_kind_not_the_database_message() {
        let (_data, app) = app();
        app.state::<crate::db::Writer>()
            .call(|c| c.execute_batch("DROP TABLE setting"))
            .unwrap();
        for (cmd, args) in [
            ("key_notation", json!({})),
            ("set_key_notation", json!({ "notation": "camelot" })),
        ] {
            let error = invoke(&app, cmd, args).unwrap_err();
            assert_eq!(error, json!({ "kind": "database", "params": {} }), "{cmd}");
            assert!(!error.to_string().contains("setting"), "{cmd}: {error}");
        }
    }

    #[test]
    fn a_failing_undo_command_sends_an_error_kind_not_the_database_message() {
        let (_data, app) = app();
        app.state::<crate::db::Writer>()
            .call(|c| c.execute_batch("ALTER TABLE operation RENAME TO gone"))
            .unwrap();
        let error = invoke(&app, "undo_last_operation", json!({})).unwrap_err();
        assert_eq!(error, json!({ "kind": "database", "params": {} }));
    }

    #[test]
    fn a_failing_job_command_sends_an_error_kind_not_the_database_message() {
        let (_data, app) = app();
        // Stop the workers first, so they don't retry against the broken
        // table while the command runs.
        app.state::<crate::jobs::JobQueue>().shutdown();
        app.state::<crate::db::Writer>()
            .call(|c| c.execute_batch("DROP TABLE job"))
            .unwrap();
        let error = invoke(&app, "cancel_job", json!({ "id": 1 })).unwrap_err();
        assert_eq!(error, json!({ "kind": "database", "params": {} }));
        assert!(!error.to_string().contains("job"), "{error}");
    }

    #[test]
    fn bindings_declare_the_error_type_and_the_key_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("errors.ts");
        crate::ipc::export_bindings(&path).unwrap();
        let ts = std::fs::read_to_string(&path).unwrap();
        for expected in [
            "export type IpcError = {",
            "kind: ErrorKind",
            "params: { [key in string]: ErrorParam }",
            "export type ErrorParam = string | number;",
            "export const ERROR_KEYS = {",
            "} as const;",
            r#"cancelJob: (id: JobId) => typedError<CancelOutcome, IpcError>("#,
            r#"activity: () => typedError<ActivitySnapshot, IpcError>("#,
            r#"undoLastOperation: () => typedError<UndoOutcome, IpcError>("#,
        ] {
            assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
        }
        assert!(
            !ts.contains(", string>(__TAURI_INVOKE"),
            "a command still fails with text:\n{ts}"
        );
        // Specta sorts the table's keys, so check each pair on its own.
        let start = ts.find("export const ERROR_KEYS = {").unwrap();
        let table = &ts[start..start + ts[start..].find(" as const;").unwrap()];
        for &kind in ErrorKind::ALL {
            let pair = format!(
                "{}:{}",
                serde_json::to_string(&kind).unwrap(),
                serde_json::to_string(kind.key()).unwrap()
            );
            assert!(table.contains(&pair), "missing {pair} in {table}");
        }
    }
}
