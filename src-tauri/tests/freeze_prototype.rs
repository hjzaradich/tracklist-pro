//! The webview freezes `Object.prototype` (0D-9), so a script that gets in
//! can't tamper with the prototypes the app's own code relies on.

use tauri::utils::config::Config;

#[test]
fn the_webview_freezes_the_object_prototype() {
    let config: Config =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses");
    assert!(config.app.security.freeze_prototype);
}
