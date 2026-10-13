//! Test helpers: the real app on Tauri's mock runtime, and commands called
//! the way the frontend calls them.

use serde_json::Value;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

/// The app with its startup hook run (it opens the database and starts the
/// job queue), its data folder in a temp dir. **Startup does not ask the
/// stages what is due**, so the queue starts quiet: a test that writes a
/// library by SQL and looks at it isn't raced by jobs the asking would queue
/// (the roles, the search index, grouping…). A test of the asking itself
/// uses [`app_asking_what_is_due`].
pub(crate) fn app() -> (tempfile::TempDir, tauri::App<MockRuntime>) {
    let data = tempfile::tempdir().unwrap();
    let app = start(data.path().join("data"), crate::StartupAsks::Skip);
    (data, app)
}

/// [`app`] on `data_dir` (which may hold a database already), with startup
/// asking the stages what is due, as the real app does. Returns once the
/// asking is over, so the jobs it queues are in the queue.
pub(crate) fn app_asking_what_is_due(data_dir: std::path::PathBuf) -> tauri::App<MockRuntime> {
    let app = start(data_dir, crate::StartupAsks::Run);
    tauri::Manager::state::<crate::jobs::startup::StartupAsks>(&app).finished();
    app
}

#[allow(deprecated)]
fn start(data_dir: std::path::PathBuf, asks: crate::StartupAsks) -> tauri::App<MockRuntime> {
    let mut app = crate::setup_with(mock_builder(), crate::DataDir::At(data_dir), asks)
        .build(mock_context(noop_assets()))
        .unwrap();
    // On the mock runtime, `run_iteration` runs the startup hook and
    // returns at once (see lib.rs).
    app.run_iteration(|_, _| {});
    app
}

/// Calls `cmd` with `args` over IPC. `Err` holds the error the frontend
/// would receive.
pub(crate) fn invoke(
    app: &tauri::App<MockRuntime>,
    cmd: &str,
    args: Value,
) -> Result<Value, Value> {
    let webview = match tauri::Manager::get_webview_window(app, "main") {
        Some(w) => w,
        None => tauri::WebviewWindowBuilder::new(app, "main", Default::default())
            .build()
            .unwrap(),
    };
    let request = tauri::webview::InvokeRequest {
        cmd: cmd.into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }
        .parse()
        .unwrap(),
        body: args.into(),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    };
    tauri::test::get_ipc_response(&webview, request).map(|body| body.deserialize().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    /// A data folder whose database says the search index is behind, as a
    /// run that died would leave it.
    fn data_with_a_search_index_behind() -> tempfile::TempDir {
        let data = tempfile::tempdir().unwrap();
        let guard = crate::write_guard::WriteGuard::app_data(data.path()).unwrap();
        let writer =
            crate::db::Writer::open(&guard.check(&crate::db::db_path(data.path())).unwrap())
                .unwrap();
        writer
            .call(|c| {
                // A music folder (the asking does nothing without one), on a
                // drive that is not here.
                c.execute_batch(
                    "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
                     INSERT INTO search_due (recording_id) VALUES (-1);",
                )
            })
            .unwrap();
        data
    }

    fn jobs_of_kind(app: &tauri::App<MockRuntime>, kind: &'static str) -> i64 {
        app.state::<crate::db::Writer>()
            .call(move |c| {
                c.query_row("SELECT count(*) FROM job WHERE kind = ?1", [kind], |r| {
                    r.get(0)
                })
            })
            .unwrap()
    }

    #[test]
    fn the_ipc_tests_app_starts_a_quiet_queue_and_never_asks_what_is_due() {
        let data = data_with_a_search_index_behind();
        let app = start(data.path().to_path_buf(), crate::StartupAsks::Skip);
        // The thread that asks was never started, so the index job a
        // started thread would have queued isn't there.
        assert!(app
            .try_state::<crate::jobs::startup::StartupAsks>()
            .is_none());
        assert_eq!(jobs_of_kind(&app, "index"), 0);
        // Nor does the plain `app()` start it.
        let (_data, plain) = super::app();
        assert!(plain
            .try_state::<crate::jobs::startup::StartupAsks>()
            .is_none());
    }

    #[test]
    fn a_test_can_opt_in_to_the_startup_asking_and_then_the_queue_has_what_was_due() {
        let data = data_with_a_search_index_behind();
        let app = app_asking_what_is_due(data.path().to_path_buf());
        assert_eq!(jobs_of_kind(&app, "index"), 1);
    }
}
