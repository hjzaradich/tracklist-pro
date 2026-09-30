fn main() {
    // Tauri embeds its Windows app manifest (Common Controls v6) into the app
    // binary only, so `cargo test` executables fail to start with
    // STATUS_ENTRYPOINT_NOT_FOUND. Embed the same manifest into every binary
    // through the linker instead, and turn off Tauri's copy.
    let is_msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut windows = tauri_build::WindowsAttributes::new();
    if is_msvc {
        // Read at run time, not with env!(): a compiled build script can be
        // reused from a shared target dir by another checkout, and a path
        // baked in at compile time would point at the wrong (or a deleted)
        // folder.
        let manifest_dir =
            std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
        let manifest = std::path::Path::new(&manifest_dir).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
