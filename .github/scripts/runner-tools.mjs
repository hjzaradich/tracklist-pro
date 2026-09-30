// Checks the owner's self-hosted runner has the tools a CI (Windows) job
// needs, and prepares the job to use them without changing them (1aA-16).
// GitHub's runners install a fresh toolchain every run; the self-hosted
// runner uses what the owner has installed, so CI never runs rustup update,
// never installs a toolchain and never touches the owner's caches. Instead
// this step prints every version into the log and fails if a tool is
// missing or isn't what CI means by it (e.g. a nightly Rust as the default,
// or a cargo-deny older than the one GitHub's runners use). The workflow
// sets RUSTUP_AUTO_INSTALL=0, so nothing makes rustup install a toolchain.
//
// Usage (in CI, self-hosted only):
//   node .github/scripts/runner-tools.mjs <need>... [--builds=<package>]...
//   needs: rust clippy rustfmt cargo-deny node npm python gh git symlinks
//   --builds: a Cargo package the job compiles; if the build folder has no
//             earlier build of it, warns that this is a slow, cold build
// First appends to $GITHUB_ENV, so the job's later steps get it even if a
// check below fails:
//   CARGO_TARGET_DIR  cargo-target next to the checkout, not in it:
//                     actions/checkout cleans ignored files in the checkout
//                     on every run, and the build output persists here
//                     instead of going through a cache. It's shared by every
//                     job and only grows, so a rust job warns above 20 GB.
// Exits 1 and lists every problem if the runner isn't ready.

import { execSync } from "node:child_process";
import { appendFileSync, existsSync, mkdtempSync, readdirSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

/** Each tool need: the command that prints its version (and a second one to print). */
export const TOOLS = {
  rust: { command: ["rustc", "--version"], also: ["cargo", "--version"] },
  clippy: { command: ["cargo", "clippy", "--version"] },
  rustfmt: { command: ["cargo", "fmt", "--version"] },
  "cargo-deny": { command: ["cargo", "deny", "--version"] },
  node: { command: ["node", "--version"] },
  npm: { command: ["npm", "--version"] },
  python: { command: ["python", "--version"] },
  gh: { command: ["gh", "--version"] },
  git: { command: ["git", "--version"] },
};

/**
 * The cargo-deny that GitHub's runners use: the one the pinned
 * cargo-deny-action bundles (deny_version in its Dockerfile at that commit).
 * A test ties `sha` to the pin in ci-windows.yml, so bumping the action means
 * looking up its cargo-deny again.
 */
export const CARGO_DENY_ACTION = { sha: "3c6349835b2b7b196a839186cb8b78e02f7b5f25", version: "0.20.2" };

/** Reads the version from `cargo deny --version` ("cargo-deny 0.20.2"), or null. */
export function cargoDenyVersion(output) {
  return output.match(/^cargo-deny (\d+\.\d+\.\d+)\b/m)?.[1] ?? null;
}

/** True if dotted version `a` is older than `b`. */
export function older(a, b) {
  const [x, y] = [a, b].map((v) => v.split(".").map(Number));
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) < (y[i] ?? 0);
  }
  return false;
}

/** Problems with the installed cargo-deny: no readable version, or older than the action's. */
export function cargoDenyProblems(output) {
  const version = cargoDenyVersion(output);
  if (!version) return ["can't read cargo-deny's version from `cargo deny --version`"];
  if (older(version, CARGO_DENY_ACTION.version)) {
    return [`cargo-deny ${version} is older than the ${CARGO_DENY_ACTION.version} GitHub's runners use; run \`cargo install --locked cargo-deny\``];
  }
  return [];
}

/** Above this, a rust job warns that the shared build folder needs a clean. */
export const TARGET_WARN_BYTES = 20e9;

/** Total size in bytes of the files under `dir` (0 if it doesn't exist). */
export function dirSize(dir) {
  if (!existsSync(dir)) return 0;
  let bytes = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) bytes += dirSize(path);
    else if (entry.isFile()) bytes += statSync(path).size;
  }
  return bytes;
}

/** The warning for a build folder of `bytes`, or null if it's small enough. */
export function targetWarning(dir, bytes) {
  if (bytes <= TARGET_WARN_BYTES) return null;
  return `The build folder ${dir} holds ${(bytes / 1e9).toFixed(1)} GB. Free the space with \`cargo clean --target-dir "${dir}"\` while no CI job runs; the next run rebuilds from scratch.`;
}

/** Needs that are checked by trying something, not by a version. */
export const CAPABILITIES = ["symlinks"];

/** Problems with the needs named on the command line (unknown names, none at all). */
export function needProblems(needs) {
  if (needs.length === 0) return ["no needs given; name the tools the job uses"];
  return needs
    .filter((n) => !(n in TOOLS) && !CAPABILITIES.includes(n))
    .map((n) => `unknown need "${n}"; known: ${[...Object.keys(TOOLS), ...CAPABILITIES].join(", ")}`);
}

/**
 * Reads `rustc -vV` output. Returns { release, channel } where channel is
 * "stable", "beta" or "nightly", or null if there's no release line.
 */
export function rustRelease(verbose) {
  const m = verbose.match(/^release:\s*(\d+\.\d+\.\d+)(?:-(beta|nightly|dev)\S*)?\s*$/m);
  if (!m) return null;
  return { release: m[1], channel: m[2] ?? "stable" };
}

/**
 * Problems with the Rust the job would use: CI means the current stable
 * (GitHub's runners install it fresh), so a beta or nightly default fails.
 */
export function rustProblems(verbose) {
  const rust = rustRelease(verbose);
  if (!rust) return ["can't read rustc's release from `rustc -vV`"];
  if (rust.channel !== "stable") {
    return [`the default Rust is ${rust.channel} ${rust.release}; CI builds with stable (rustup default stable)`];
  }
  return [];
}

/**
 * Reads `rustup check` output and returns the stable update it offers, as
 * { from, to }, or null if stable is up to date or isn't listed.
 * e.g. "stable-x86_64-pc-windows-msvc - update available: 1.98.0 (88d9e12ae 2026-08-18) -> 1.98.1 (…)";
 * older rustups print "Update available :".
 */
export function stableUpdate(check) {
  const m = check.match(/^stable-\S+ - update available\s*:\s*(\d+\.\d+\.\d+)\b.*?->\s*(\d+\.\d+\.\d+)\b/im);
  return m ? { from: m[1], to: m[2] } : null;
}

/**
 * The build folder for a checkout at `workspace` ($GITHUB_WORKSPACE, e.g.
 * _work/tracklist-pro/tracklist-pro): its sibling, in the runner's folder
 * for this repo.
 */
export function targetDir(workspace) {
  return join(dirname(workspace), "cargo-target");
}

/** Splits the command line into needs and --builds= packages. */
export function parseArgs(args) {
  const needs = [];
  const builds = [];
  const problems = [];
  for (const arg of args) {
    if (arg.startsWith("--builds=") && arg.length > "--builds=".length) builds.push(arg.slice("--builds=".length));
    else if (arg.startsWith("-")) problems.push(`unknown option "${arg}"; the only option is --builds=<package>`);
    else needs.push(arg);
  }
  return { needs, builds, problems };
}

/**
 * True if the build folder has no earlier build of `pkg`: cargo keeps a
 * `<package>-<hash>` folder under debug/.fingerprint for each build of it.
 */
export function isCold(dir, pkg) {
  const fingerprints = join(dir, "debug", ".fingerprint");
  if (!existsSync(fingerprints)) return true;
  return !readdirSync(fingerprints).some((name) => name.startsWith(`${pkg}-`));
}

/** The warning for a cold build of `pkg`. */
export function coldWarning(pkg) {
  return `No earlier build of ${pkg} in the build folder, so this job compiles everything from scratch. On a busy PC that can take over an hour; later runs reuse the build.`;
}

/** The lines this step appends to $GITHUB_ENV. */
export function envLines(workspace) {
  return [`CARGO_TARGET_DIR=${targetDir(workspace)}`];
}

// Through the shell, so Windows finds npm.cmd and the like as a job's step
// would. Every command here is a fixed literal from TOOLS.
function run(command) {
  return execSync(command.join(" "), { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

/** True if this user can make a file symlink (Windows needs Developer Mode or admin). */
export function canSymlink() {
  const dir = mkdtempSync(join(tmpdir(), "runner-tools-"));
  try {
    writeFileSync(join(dir, "target"), "");
    symlinkSync(join(dir, "target"), join(dir, "link"), "file");
    return true;
  } catch {
    return false;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function main(args) {
  // Before anything can fail, so later steps never build inside the checkout.
  const workspace = process.env.GITHUB_WORKSPACE ?? process.cwd();
  if (process.env.GITHUB_ENV) {
    const lines = envLines(workspace);
    appendFileSync(process.env.GITHUB_ENV, lines.map((l) => `${l}\n`).join(""));
    console.log(lines.join("\n"));
  }
  const { needs, builds, problems } = parseArgs(args);
  problems.push(...needProblems(needs));
  if (problems.length > 0) {
    for (const p of problems) console.error(`::error::${p}`);
    process.exit(2);
  }
  for (const need of needs) {
    const tool = TOOLS[need];
    if (!tool) continue;
    for (const command of [tool.command, tool.also].filter(Boolean)) {
      let version;
      try {
        version = run(command).trim().split(/\r?\n/)[0];
      } catch {
        problems.push(`${need}: \`${command.join(" ")}\` failed; install it for the runner's user`);
        continue;
      }
      console.log(`${command.join(" ")}: ${version}`);
      if (need === "cargo-deny") problems.push(...cargoDenyProblems(version));
    }
  }
  if (needs.includes("rust")) {
    try {
      problems.push(...rustProblems(run(["rustc", "-vV"])));
    } catch {
      // Already reported as missing above.
    }
    // Only a warning: GitHub's runners would use the newer stable (and its
    // new lints), but updating is the owner's call. Needs the network, so a
    // failure to check is only a note.
    let check = null;
    try {
      check = run(["rustup", "check"]);
    } catch (e) {
      // rustup check exits 100 when an update is available.
      if (e.status === 100) check = e.stdout;
    }
    const update = check === null ? null : stableUpdate(check);
    if (check === null) console.log("rustup check failed; can't tell whether stable Rust is current.");
    else if (update) {
      console.log(`::warning::The runner's stable Rust is ${update.from}; ${update.to} is out. Run \`rustup update stable\` so CI lints with the current stable, as GitHub's runners do.`);
    } else console.log("rustup check: stable Rust is current.");
    const dir = targetDir(workspace);
    const bytes = dirSize(dir);
    console.log(`build folder: ${dir}, ${(bytes / 1e9).toFixed(1)} GB`);
    const warning = targetWarning(dir, bytes);
    if (warning) console.log(`::warning::${warning}`);
  }
  for (const pkg of builds) {
    if (isCold(targetDir(workspace), pkg)) console.log(`::warning::${coldWarning(pkg)}`);
    else console.log(`build folder: has an earlier build of ${pkg}`);
  }
  if (needs.includes("symlinks")) {
    if (canSymlink()) console.log("symlinks: this user can create file symlinks");
    else {
      problems.push(
        "symlinks: this user can't create file symlinks, so the write guard's symlink tests would skip. Turn on Developer Mode (Settings > System > For developers).",
      );
    }
  }
  if (problems.length > 0) {
    for (const p of problems) console.error(`::error::${p}`);
    process.exit(1);
  }
  console.log("The runner has every tool this job needs.");
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
