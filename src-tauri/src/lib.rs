use std::path::PathBuf;

use tauri::{Builder, Manager, RunEvent, Runtime};

pub mod after_send;
pub mod all_music;
pub mod attach;
pub mod crates;
pub mod db;
pub mod fingerprint;
pub mod fragile;
pub mod grouping;
pub mod hash;
pub mod ipc;
pub mod jobs;
pub mod library;
pub mod matching;
pub mod missing;
pub mod net;
pub mod ops;
pub mod paths;
pub mod read;
pub mod rekordbox;
pub mod rekordbox_write;
pub mod relink;
pub mod scan;
pub mod scan_state;
pub mod send;
pub mod send_values;
pub mod settings;
pub mod sniff;
pub mod start;
pub mod suggest;
pub mod tags;
pub mod volume;
pub mod write_guard;

/// The bundle ID. It also names the app data folder,
/// `%APPDATA%\com.tracklistpro.desktop\` (ROADMAP §1.1).
pub const IDENTIFIER: &str = "com.tracklistpro.desktop";

/// Which folder the app keeps its data in.
enum DataDir {
    /// The real one, named after the bundle ID.
    AppData,
    /// Somewhere else. Tests use a temp dir so they never touch the real
    /// database.
    At(PathBuf),
}

/// Resolves the app data folder without creating it.
fn resolve_data_dir<R: Runtime, M: Manager<R>>(
    app: &M,
    data_dir: &DataDir,
) -> tauri::Result<PathBuf> {
    match data_dir {
        DataDir::AppData => app.path().app_data_dir(),
        DataDir::At(dir) => Ok(dir.clone()),
    }
}

/// Everything the app does at startup, shared by `run` and the tests.
///
/// Creates the app data folder through the write guard, opens the
/// database's one writer connection there, and hands both to Tauri's state,
/// where commands reach them as `State<WriteGuard>` and `State<db::Writer>`,
/// keeps where the send flow is (`State<send::SendFlow>`),
/// starts the job queue (`State<jobs::JobQueue>`) and the music folder
/// watchers (`State<scan::Watchers>`), then opens the window, kept on the
/// app's own pages (`net::navigation`).
fn setup<R: Runtime>(builder: Builder<R>, data_dir: DataDir) -> Builder<R> {
    let specta = ipc::specta_builder::<R>();
    builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            let dir = resolve_data_dir(app, &data_dir)?;
            let guard = write_guard::WriteGuard::app_data(&dir)?;
            let writer = db::Writer::open(&guard.check(&db::db_path(guard.app_data_dir()))?)?;
            app.manage(db::ReadPool::open(writer.guarded_path())?);
            scan::volumes::start(app.handle(), &writer);
            // Before the job queue: send jobs an earlier run left are ended
            // (the handler ignores them in any case: they name that run),
            // and the send job's handler shares the flow.
            writer.call(|c| send::drop_unfinished_jobs(c))?;
            app.manage(send::SendFlow::new(guard.clone()));
            app.manage(jobs::start(app.handle(), writer.clone())?);
            app.manage(scan::watch::start(app.handle(), writer.clone()));
            app.manage(writer);
            app.manage(guard);
            app.manage(rekordbox::source::ExportFolder::new(
                app.path().document_dir().ok(),
            ));
            net::navigation::open_windows(app)?;
            Ok(())
        })
}

/// The app's real startup with its data folder at `data_dir` instead of
/// `%APPDATA%`, for the integration tests in `tests/`: they build it on
/// Tauri's mock runtime, so what they drive is what [`run`] starts.
/// Nothing in the app calls this.
#[doc(hidden)]
pub fn setup_for_tests<R: Runtime>(builder: Builder<R>, data_dir: PathBuf) -> Builder<R> {
    setup(builder, DataDir::At(data_dir))
}

pub fn run() {
    setup(Builder::default(), DataDir::AppData)
        .build(tauri::generate_context!())
        .expect("error while building tracklist-pro")
        .run(on_run_event);
}

/// Handles the app's lifecycle events. On exit (the process ends right
/// after, without dropping Tauri's state) it stops the music folder
/// watchers, so no burst queues a scan behind the queue's back, then the
/// job queue, so running jobs go back in the queue for the next launch.
fn on_run_event<R: Runtime>(app: &tauri::AppHandle<R>, event: RunEvent) {
    if let RunEvent::Exit = event {
        if let Some(watchers) = app.try_state::<scan::Watchers>() {
            watchers.shutdown();
        }
        if let Some(jobs) = app.try_state::<jobs::JobQueue>() {
            jobs.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::Path;
    use tauri::test::{mock_builder, MockRuntime};
    use tauri::Listener;

    const MUSICMANAGER_IDENTIFIER: &str = "com.musicmanager.desktop";

    fn config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap()
    }

    /// Builds the app on Tauri's mock runtime with the real, compiled-in
    /// config and the real startup hook, which runs only on `run_iteration`.
    pub(crate) fn mock_app_with(data_dir: DataDir) -> tauri::App<MockRuntime> {
        // `test = true`: on macOS a second, non-test context would embed
        // Info.plist again and fail to link (duplicate `_EMBED_INFO_PLIST`).
        setup(mock_builder(), data_dir)
            .build(tauri::generate_context!(test = true))
            .unwrap()
    }

    /// The app with its data folder in a fresh temp dir, so no test can
    /// create or open the real database in %APPDATA%. Keep the returned
    /// `TempDir` alive as long as the app. Tests about the real folder ask
    /// `app.path().app_data_dir()`, which this doesn't change.
    fn mock_app() -> (tempfile::TempDir, tauri::App<MockRuntime>) {
        let data = tempfile::tempdir().unwrap();
        let app = mock_app_with(DataDir::At(data.path().join(IDENTIFIER)));
        (data, app)
    }

    fn is_onedrive(path: &Path) -> bool {
        let lower = |s: &OsStr| s.to_string_lossy().to_lowercase();
        let by_name = path
            .components()
            .any(|c| lower(c.as_os_str()).starts_with("onedrive"));
        // OneDrive can be relocated and renamed; Windows publishes its roots here.
        let by_env = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
            .iter()
            .filter_map(std::env::var_os)
            .any(|root| lower(path.as_os_str()).starts_with(&lower(&root)));
        by_name || by_env
    }

    #[test]
    fn config_identifier_is_tracklist_pro() {
        assert_eq!(config()["identifier"], IDENTIFIER);
        assert_eq!(IDENTIFIER, "com.tracklistpro.desktop");
    }

    #[test]
    fn config_identifier_is_not_musicmanager() {
        assert_ne!(config()["identifier"], MUSICMANAGER_IDENTIFIER);
    }

    #[test]
    fn config_product_name_is_tracklist_pro() {
        assert_eq!(config()["productName"], "tracklist-pro");
    }

    #[test]
    fn built_app_uses_tracklist_pro_identifier() {
        let (_data, app) = mock_app();
        assert_eq!(app.config().identifier, IDENTIFIER);
        assert_ne!(app.config().identifier, MUSICMANAGER_IDENTIFIER);
        assert_eq!(app.package_info().name, "tracklist-pro");
    }

    #[test]
    fn data_dir_ends_in_identifier_and_is_not_under_onedrive() {
        let (_data, app) = mock_app();
        let dir = app.path().app_data_dir().unwrap();
        assert!(dir.ends_with(IDENTIFIER), "data dir {dir:?}");
        assert!(!is_onedrive(&dir), "data dir {dir:?} is under OneDrive");
    }

    #[cfg(windows)]
    #[test]
    fn data_dir_is_in_roaming_appdata() {
        let (_data, app) = mock_app();
        let dir = app.path().app_data_dir().unwrap();
        let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap());
        assert_eq!(dir, appdata.join(IDENTIFIER));
    }

    #[test]
    fn real_startup_uses_the_bundle_id_data_folder() {
        let (_data, app) = mock_app();
        let resolved = resolve_data_dir(&app, &DataDir::AppData).unwrap();
        assert_eq!(resolved, app.path().app_data_dir().unwrap());
        assert!(resolved.ends_with(IDENTIFIER), "data dir {resolved:?}");
    }

    #[test]
    fn db_path_is_inside_the_data_folder_and_not_under_onedrive() {
        let (_data, app) = mock_app();
        let data_dir = app.path().app_data_dir().unwrap();
        let path = db::db_path(&data_dir);
        assert!(
            path.starts_with(&data_dir),
            "db {path:?} is outside {data_dir:?}"
        );
        assert!(!is_onedrive(&path), "db {path:?} is under OneDrive");
    }

    #[test]
    #[allow(deprecated)]
    fn startup_creates_data_dir_and_opens_the_db_writer_there() {
        // Tauri runs the startup hook when the event loop starts. On the mock
        // runtime, `run_iteration` runs that same hook and returns at once
        // (`run` would loop forever). A temp data folder stands in for the
        // real one, so the real database is never touched.
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("nested").join(IDENTIFIER);
        let mut app = mock_app_with(DataDir::At(dir.clone()));
        app.run_iteration(|_, _| {});
        assert!(dir.is_dir(), "data dir {dir:?} was not created");

        // The guard holds the folder as it really is on disk, and the
        // database is opened at that spelling.
        let real_dir = std::fs::canonicalize(&dir).unwrap();
        let guard = app.state::<write_guard::WriteGuard>();
        assert_eq!(guard.app_data_dir(), real_dir);
        let writer = app.state::<db::Writer>();
        assert_eq!(writer.path(), db::db_path(&real_dir));
        assert!(
            writer.path().is_file(),
            "db {:?} was not created",
            writer.path()
        );
        let one: i64 = writer
            .call(|c| c.query_row("SELECT 1", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(one, 1);

        let reader = app.state::<db::ReadPool>();
        assert_eq!(reader.path(), writer.path());
        let migrated = reader.read(db::migrations::applied).unwrap();
        assert_eq!(migrated.len(), db::migrations::MIGRATIONS.len());
    }

    #[test]
    #[allow(deprecated)]
    fn startup_opens_the_configured_window_through_the_navigation_guard() {
        // tauri.conf.json marks the window `"create": false`; startup opens
        // it itself, with the guard (net::navigation). The mock runtime
        // never navigates, so the guard's decisions are tested in
        // net::navigation; this proves the window still opens.
        let (_data, mut app) = mock_app();
        assert!(app.get_webview_window("main").is_none());
        app.run_iteration(|_, _| {});
        assert!(
            app.get_webview_window("main").is_some(),
            "startup opens the main window"
        );
        assert_eq!(app.webview_windows().len(), 1);
    }

    #[test]
    fn app_info_command_answers_over_ipc_with_camel_case_fields() {
        let (_data, app) = mock_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let request = tauri::webview::InvokeRequest {
            cmd: "app_info".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        let body = tauri::test::get_ipc_response(&webview, request).unwrap();
        let json: serde_json::Value = body.deserialize().unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "tracklist-pro",
                "version": env!("CARGO_PKG_VERSION"),
                "identifier": IDENTIFIER,
            })
        );
    }

    /// Calls `cmd` over IPC, as the frontend does, and returns its JSON answer.
    fn invoke(
        webview: &tauri::WebviewWindow<MockRuntime>,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, serde_json::Value> {
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
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(webview, request).map(|body| body.deserialize().unwrap())
    }

    /// The app after its startup hook has run, and its main window.
    #[allow(deprecated)]
    fn started_app() -> (
        tempfile::TempDir,
        tauri::App<MockRuntime>,
        tauri::WebviewWindow<MockRuntime>,
    ) {
        let data = tempfile::tempdir().unwrap();
        let mut app = mock_app_with(DataDir::At(data.path().join(IDENTIFIER)));
        app.run_iteration(|_, _| {});
        // Startup opened the main window from the config.
        let webview = app.get_webview_window("main").unwrap();
        (data, app, webview)
    }

    #[test]
    fn startup_starts_the_job_queue_and_the_frontend_can_ask_what_is_running() {
        let (_data, app, webview) = started_app();
        assert!(app.try_state::<jobs::JobQueue>().is_some());
        let snapshot = invoke(&webview, "activity", serde_json::json!({})).unwrap();
        assert_eq!(snapshot, serde_json::json!({ "seq": 0, "jobs": [] }));
    }

    #[test]
    fn the_frontend_can_cancel_a_queued_job() {
        let (_data, app, webview) = started_app();
        let queue = app.state::<jobs::JobQueue>();
        // Stop the workers, so the job stays queued until it's cancelled.
        queue.shutdown();
        let id = queue
            .enqueue(jobs::NewJob::new(jobs::JobKind::Export))
            .unwrap();
        let answer = invoke(&webview, "cancel_job", serde_json::json!({ "id": id.0 }));
        assert_eq!(answer, Ok(serde_json::json!("cancelled")));
        let again = invoke(&webview, "cancel_job", serde_json::json!({ "id": id.0 }));
        assert_eq!(again, Ok(serde_json::json!("not_active")));
        let job = app
            .state::<db::Writer>()
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        assert_eq!(job.status, jobs::JobStatus::Cancelled);
    }

    #[test]
    fn a_row_id_past_32_bits_round_trips_over_ipc_exactly() {
        let (_data, app, webview) = started_app();
        let queue = app.state::<jobs::JobQueue>();
        // Stop the workers, so the jobs stay queued.
        queue.shutdown();
        let big: i64 = 1 << 40;
        app.state::<db::Writer>()
            .call(move |c| c.execute("INSERT INTO job (id, kind) VALUES (?1, 'export')", [big]))
            .unwrap();

        // Frontend to Rust: the id arrives exactly.
        let answer = invoke(&webview, "cancel_job", serde_json::json!({ "id": big }));
        assert_eq!(answer, Ok(serde_json::json!("cancelled")));
        let cancelled = app
            .state::<db::Writer>()
            .call(move |c| jobs::store::get(c, jobs::JobId(big)))
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.status, jobs::JobStatus::Cancelled);

        // Rust to frontend: the next id, 2^40 + 1, arrives exactly.
        let next = queue
            .enqueue(jobs::NewJob::new(jobs::JobKind::Export))
            .unwrap();
        assert_eq!(next, jobs::JobId(big + 1));
        let snapshot = invoke(&webview, "activity", serde_json::json!({})).unwrap();
        assert_eq!(
            snapshot["jobs"][0]["id"],
            serde_json::json!(1_099_511_627_777_i64)
        );
    }

    #[test]
    fn job_progress_reaches_the_frontend_as_typed_job_updates_events() {
        use std::sync::{mpsc, Mutex};
        use tauri_specta::Event;

        let (_data, app, _webview) = started_app();
        let (heard, batches) = mpsc::channel();
        let heard = Mutex::new(heard);
        // What a webview hears: the event by its registered name, as JSON.
        app.listen_any("job-updates", move |event| {
            let json: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            let _ = heard.lock().unwrap().send(json);
        });
        // The typed listener agrees on the name and the shape.
        let (typed_tx, typed) = mpsc::channel();
        let typed_tx = Mutex::new(typed_tx);
        jobs::JobUpdates::listen_any(&app, move |event| {
            let _ = typed_tx.lock().unwrap().send(event.payload);
        });

        // The app's own queue has no handlers yet; stop it so it can't take
        // this test's job.
        app.state::<jobs::JobQueue>().shutdown();
        let writer = app.state::<db::Writer>().inner().clone();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let queue = jobs::JobQueue::builder(writer)
            .on_updates(jobs::emitter(app.handle()))
            .handler(jobs::JobKind::Scan, move |job: &jobs::JobContext| {
                job.progress(0.5)?;
                // Hold here, so the 50% update goes out on its own.
                let _ = gate
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10));
                Ok(())
            })
            .start()
            .unwrap();
        let id = queue
            .enqueue(jobs::NewJob::new(jobs::JobKind::Scan))
            .unwrap();

        let patience = std::time::Duration::from_secs(10);
        let mut seen: Vec<serde_json::Value> = Vec::new();
        let status = |u: &serde_json::Value| u["status"].as_str().unwrap().to_owned();
        while seen.last().map(status).as_deref() != Some("running")
            || seen.last().unwrap()["progress"].is_null()
        {
            let batch = batches.recv_timeout(patience).unwrap();
            seen.extend(batch.as_array().unwrap().iter().cloned());
        }
        assert_eq!(
            seen.last().unwrap(),
            &serde_json::json!({
                "seq": 3, "id": id.0, "kind": "scan", "status": "running",
                "progress": 0.5, "priority": 0,
            })
        );
        release.send(()).unwrap();
        while seen.last().map(status).as_deref() != Some("done") {
            let batch = batches.recv_timeout(patience).unwrap();
            seen.extend(batch.as_array().unwrap().iter().cloned());
        }
        assert_eq!(seen.last().unwrap()["progress"], serde_json::json!(1.0));
        // Batches may merge steps, but never reorder them.
        let seqs: Vec<_> = seen.iter().map(|u| u["seq"].as_u64().unwrap()).collect();
        assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
        let first = typed.recv_timeout(patience).unwrap();
        assert_eq!(first.0[0].id, id);
    }

    #[test]
    fn exiting_the_app_stops_the_job_queue() {
        let (_data, app, _webview) = started_app();
        on_run_event(app.handle(), RunEvent::Exit);
        // With the workers still running, this job would be taken at once
        // and finish; after exit it stays queued.
        let queue = app.state::<jobs::JobQueue>();
        let id = queue
            .enqueue(jobs::NewJob::new(jobs::JobKind::Scan))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let job = app
            .state::<db::Writer>()
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        assert_eq!(job.status, jobs::JobStatus::Queued);
    }

    #[test]
    fn without_the_exit_hook_the_same_job_would_run() {
        // The control for the test above: a live queue takes the job.
        let (_data, app, _webview) = started_app();
        let queue = app.state::<jobs::JobQueue>();
        let id = queue
            .enqueue(jobs::NewJob::new(jobs::JobKind::Scan))
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            let job = app
                .state::<db::Writer>()
                .call(move |c| jobs::store::get(c, id))
                .unwrap()
                .unwrap();
            // Scan has a handler now: with no music folders it's done at once.
            if job.status.is_finished() {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[cfg(windows)]
    #[test]
    fn is_onedrive_detects_onedrive_paths() {
        assert!(is_onedrive(Path::new(
            r"C:\Users\someone\OneDrive\Desktop\x"
        )));
        assert!(is_onedrive(Path::new(
            r"C:\Users\someone\OneDrive - Contoso\x"
        )));
        assert!(!is_onedrive(Path::new(
            r"C:\Users\someone\AppData\Roaming\x"
        )));
    }
}
