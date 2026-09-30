//! The generator is a dev tool: the app never depends on it, so it can't end
//! up in a release build.

#[test]
fn the_app_does_not_depend_on_the_fixture_generator() {
    let manifest = include_str!("../../../src-tauri/Cargo.toml");
    let lock = include_str!("../../../src-tauri/Cargo.lock");
    for text in [manifest, lock] {
        assert!(!text.contains("fixture-gen"));
        assert!(!text.contains("fixture_gen"));
    }
}
