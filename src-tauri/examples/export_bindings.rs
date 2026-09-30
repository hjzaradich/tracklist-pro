//! Regenerates the TypeScript bindings at `src/bindings.ts`.
//!
//! Run from `src-tauri/`: `cargo run --example export_bindings`.
//! Pass a path to write somewhere else (e.g. to diff against the committed file).

use std::path::PathBuf;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(tracklist_pro_lib::ipc::bindings_path);
    tracklist_pro_lib::ipc::export_bindings(&path).expect("failed to export TypeScript bindings");
    println!("wrote {}", path.display());
}
