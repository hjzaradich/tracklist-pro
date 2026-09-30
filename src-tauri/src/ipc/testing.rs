//! Test helpers: the real app on Tauri's mock runtime, and commands called
//! the way the frontend calls them.

use serde_json::Value;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

/// The app with its startup hook run (it opens the database and starts the
/// job queue), its data folder in a temp dir.
#[allow(deprecated)]
pub(crate) fn app() -> (tempfile::TempDir, tauri::App<MockRuntime>) {
    let data = tempfile::tempdir().unwrap();
    let mut app = crate::setup(mock_builder(), crate::DataDir::At(data.path().join("data")))
        .build(mock_context(noop_assets()))
        .unwrap();
    // On the mock runtime, `run_iteration` runs the startup hook and
    // returns at once (see lib.rs).
    app.run_iteration(|_, _| {});
    (data, app)
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
