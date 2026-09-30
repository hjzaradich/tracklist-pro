//! `fixture-gen --out <empty folder> [--seed N] [--files N] [--bulk-ms N] [--threads N]`
//!
//! Writes a synthetic music folder and `fixture-manifest.json` into the
//! folder given, and nowhere else. Dev-only; see README.md.

use std::path::PathBuf;
use std::process::ExitCode;

use fixture_gen::{generate, Options};

const USAGE: &str = "\
usage: fixture-gen --out <folder> [--seed N] [--files N] [--bulk-ms N] [--threads N]

  --out      where to write; must be new or empty (its parent must exist)
  --seed     same seed, same bytes (default 1)
  --files    total files, ground-truth cases included; 0 = those cases only (default 0)
  --bulk-ms  length of each bulk file in ms (default 500)
  --threads  worker threads, 0 = one per core (default 0)";

fn parse() -> Result<(PathBuf, Options), String> {
    let mut out = None;
    let mut opts = Options::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        let flag = flag.to_string_lossy().into_owned();
        if flag == "-h" || flag == "--help" {
            return Err(String::new());
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let number = || -> Result<u64, String> {
            value
                .to_str()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{flag} needs a number"))
        };
        match flag.as_str() {
            "--out" => out = Some(PathBuf::from(&value)),
            "--seed" => opts.seed = number()?,
            "--files" => opts.files = number()? as usize,
            "--bulk-ms" => opts.bulk_ms = number()?.clamp(50, 60_000) as u32,
            "--threads" => opts.threads = number()? as usize,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let out = out.ok_or("--out is required")?;
    Ok((out, opts))
}

fn main() -> ExitCode {
    let (out, opts) = match parse() {
        Ok(v) => v,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match generate(&out, &opts) {
        Ok(s) => {
            println!(
                "wrote {} files, {:.1} MB, in {:.1} s, to {}",
                s.files,
                s.bytes as f64 / 1_000_000.0,
                s.elapsed.as_secs_f64(),
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("fixture-gen: {e}");
            ExitCode::FAILURE
        }
    }
}
