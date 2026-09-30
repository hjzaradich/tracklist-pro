//! Prints the volume each path lives on, for checking real drives by hand.
//!
//! Run from `src-tauri/`: `cargo run --example volume_probe -- <path> [<path>...]`
//! e.g. `cargo run --example volume_probe -- C:\ E:\ Z:\Music`
//! Read-only: it only asks Windows about the volume, it never writes.

use std::path::Path;
use std::process::ExitCode;

use tracklist_pro_lib::volume::{volume_for, VolumeKind};

fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: cargo run --example volume_probe -- <path> [<path>...]");
        return ExitCode::FAILURE;
    }
    let mut ok = true;
    for path in &paths {
        println!("{path}");
        match volume_for(Path::new(path)) {
            Ok(volume) => {
                let kind = match volume.kind {
                    VolumeKind::Internal => "internal",
                    VolumeKind::External => "external",
                    VolumeKind::Network => "network",
                };
                println!("  id:         {}", volume.id);
                println!("  label:      {}", volume.label);
                println!("  mount path: {}", volume.mount_path.display());
                println!("  kind:       {kind}");
            }
            Err(e) => {
                println!("  error:      {e}");
                ok = false;
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
