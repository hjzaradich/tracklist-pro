// Guard for ci-changes.mjs's frontend list (1cC-D2). A pull request that
// changes only frontend files skips the Rust job, so every frontend file a
// Rust test reads must be in RUST_READS, or a change to it would silently
// stop being tested. This scans src-tauri/**/*.rs for reads that reach
// outside src-tauri and fails if one lands on a path that wouldn't run the
// Rust job. Run with: node --test .github/scripts/
//
// What the scan recognizes (a heuristic on the source text, not a Rust
// parser; a false hit costs nothing unless the path is frontend-only):
//   1. include_str!("rel") / include_bytes!("rel"): relative to the .rs file.
//   2. concat!(env!("CARGO_MANIFEST_DIR"), "/rel"): relative to src-tauri/.
//   3. any other string literal starting "../" or "/../" (.join("../src/x"),
//      a const such as BINDINGS_REL_PATH): relative to src-tauri/.
//   4. .join("..").join("a").join("b"): relative to src-tauri/.
//   5. manifest_dir().parent() / env!("CARGO_MANIFEST_DIR").parent() followed
//      by .join("x") calls: relative to the repo root. When the root is kept
//      in a variable (no join straight after), every .join("name") in that
//      file that names a file at the repo root counts too.
// A path that doesn't exist (or is outside the repo, or inside src-tauri) is
// not a read of anything. A folder is read as "any file in it, even a new
// one", so the folder must be covered whole.
//
// NOT found: a path built from pieces at run time, or a root held in a
// variable and joined a name that is not a root file. A new reader written
// that way needs adding to RUST_READS (or SHARED) by hand.

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { jobsFor, isFrontendOnly, isRustRead } from "./ci-changes.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** Every .rs file under src-tauri (build output excluded): { path (repo-relative, "/"), text }. */
function rustSources(root = repoRoot) {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== "target" && entry.name !== "node_modules") walk(full);
      } else if (entry.name.endsWith(".rs")) {
        out.push({ path: relative(root, full).split(sep).join("/"), text: readFileSync(full, "utf8") });
      }
    }
  };
  walk(join(root, "src-tauri"));
  return out;
}

const strip = (rel) => rel.replace(/^\/+/, "");
const joins = (text) => [...text.matchAll(/\.join\(\s*"([^"]*)"\s*\)/g)].map((m) => m[1]);

/**
 * The repo-relative paths a Rust source file reads from outside src-tauri.
 * Returns [{ literal (what the source says), path (repo-relative) }]; a folder's
 * path ends in "/".
 */
export function outsideReads({ path: file, text }, root = repoRoot) {
  const candidates = []; // { literal, abs }
  const fromFile = (rel) => resolve(root, dirname(file), rel);
  const fromManifest = (rel) => resolve(root, "src-tauri", strip(rel));
  const fromRoot = (rel) => resolve(root, strip(rel));

  // 1. include macros. Taken out of the text afterwards so form 3 doesn't read
  // the same literal against the wrong base.
  const rest = text.replace(/include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)/g, (_, rel) => {
    candidates.push({ literal: rel, abs: fromFile(rel) });
    return "";
  });
  // 2.
  for (const m of rest.matchAll(/concat!\(\s*env!\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*,\s*"([^"]*)"/g)) {
    candidates.push({ literal: m[1], abs: fromManifest(m[1]) });
  }
  // 3.
  for (const m of rest.matchAll(/"(\/?\.\.\/[^"]*)"/g)) {
    candidates.push({ literal: m[1], abs: fromManifest(m[1]) });
  }
  // 4.
  for (const m of rest.matchAll(/((?:\.join\(\s*"\.\."\s*\)\s*)+)((?:\.join\(\s*"[^"]*"\s*\)\s*)+)/g)) {
    const ups = joins(m[1]).length;
    const rel = [...Array(ups).fill(".."), ...joins(m[2])].join("/");
    candidates.push({ literal: rel, abs: fromManifest(rel) });
  }
  // 5.
  let rootVariable = false;
  const parent = /(?:manifest_dir\(\)|env!\(\s*"CARGO_MANIFEST_DIR"\s*\))\s*\.parent\(\)\s*\.unwrap\(\)((?:\s*\.join\(\s*"[^"]*"\s*\))*)/g;
  for (const m of rest.matchAll(parent)) {
    const names = joins(m[1]);
    if (names.length === 0) rootVariable = true;
    else candidates.push({ literal: names.join("/"), abs: fromRoot(names.join("/")) });
  }
  if (rootVariable) {
    for (const name of joins(rest)) {
      const abs = fromRoot(name);
      if (existsSync(abs) && statSync(abs).isFile()) candidates.push({ literal: name, abs });
    }
  }

  const reads = [];
  for (const { literal, abs } of candidates) {
    const rel = relative(root, abs).split(sep).join("/");
    if (rel === "" || rel.startsWith("..") || rel === "src-tauri" || rel.startsWith("src-tauri/")) continue;
    if (!existsSync(abs)) continue;
    const path = statSync(abs).isDirectory() ? `${rel}/` : rel;
    if (!reads.some((r) => r.path === path)) reads.push({ literal, path });
  }
  return reads;
}

/**
 * The reads that wouldn't run the Rust job when they change.
 * `needsRust(path)` says whether a change to `path` runs it.
 */
export function uncoveredReads(sources, { root = repoRoot, needsRust = (p) => jobsFor([p]).rust } = {}) {
  const bad = [];
  for (const source of sources) {
    for (const read of outsideReads(source, root)) {
      // A folder is read whole: a new file in it must run the job too.
      const probe = read.path.endsWith("/") ? `${read.path}__new_file__` : read.path;
      if (!needsRust(probe)) bad.push({ file: source.path, ...read });
    }
  }
  return bad;
}

const describe = (bad) =>
  bad.map((b) => `${b.file} reads ${b.path} (${b.literal}); add it to RUST_READS (frontend files) or SHARED in ci-changes.mjs`).join("\n");

test("every file or folder src-tauri reads from outside it runs the Rust job when it changes", () => {
  const sources = rustSources();
  assert.ok(sources.length > 100, `found only ${sources.length} Rust files; is the scan looking in the right place?`);
  const bad = uncoveredReads(sources);
  assert.deepEqual(bad, [], `\n${describe(bad)}\n`);
});

test("the scan finds the readers we know of, so an empty result can't mean it saw nothing", () => {
  const found = new Set();
  for (const source of rustSources()) {
    for (const read of outsideReads(source)) found.add(`${source.path} -> ${read.path}`);
  }
  const known = [
    ["src-tauri/src/ipc.rs", "src/bindings.ts"],
    ["src-tauri/src/ipc/error.rs", "src/locales/en/"],
    ["src-tauri/src/review/reasons.rs", "src/locales/en/review.json"],
    ["src-tauri/src/review/tests/plan.rs", "src/locales/en/review.json"],
    ["src-tauri/src/preview.rs", "scripts/preview.mjs"],
    ["src-tauri/src/rekordbox/tests.rs", "spikes/rb-kit/fixtures/rb-export-snippet.xml"],
    ["src-tauri/src/write_guard/scan.rs", "tools/"],
    ["src-tauri/src/write_guard/scan.rs", "package.json"],
    ["src-tauri/tests/net_gate.rs", "deny.toml"],
  ];
  for (const [file, path] of known) assert.ok(found.has(`${file} -> ${path}`), `not found: ${file} reads ${path}`);
});

test("RUST_READS entries run the Rust job, and a frontend file nobody reads does not", () => {
  for (const path of ["src/bindings.ts", "src/locales/en/review.json", "src/locales/en/errors.json", "scripts/preview.mjs"]) {
    assert.ok(isRustRead(path), path);
    assert.ok(!isFrontendOnly(path), path);
    assert.ok(jobsFor([path]).rust, path);
  }
  assert.ok(isFrontendOnly("src/main.tsx"));
  assert.ok(isFrontendOnly("src/locales/notes/review.json"));
  assert.ok(!jobsFor(["src/main.tsx"]).rust);
});

// Proof that the guard fails for an unlisted reader.

const reader = (text, path = "src-tauri/src/review/tests/x.rs") => ({ path, text });

test("a new Rust test reading an unlisted frontend file fails the guard", () => {
  // src/main.tsx and src/theme exist and are frontend-only: nothing in
  // RUST_READS covers them.
  const cases = {
    "include_str!": 'let s = include_str!("../../../../src/main.tsx");',
    "include_bytes!": 'let s = include_bytes!("../../../../src/main.tsx");',
    "CARGO_MANIFEST_DIR concat": 'let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/main.tsx");',
    "a ../ literal": 'let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/main.tsx");',
    "a .join(..) chain": 'let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("src").join("main.tsx");',
    "a folder": 'let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/theme");',
    "the repo root's child": 'let p = manifest_dir().parent().unwrap().join("src").join("main.tsx");',
    "a root variable": 'let root = manifest_dir().parent().unwrap().to_path_buf();\nlet p = root.join("index.html");',
  };
  for (const [name, text] of Object.entries(cases)) {
    const bad = uncoveredReads([reader(text)]);
    assert.equal(bad.length, 1, `${name}: expected one uncovered read, got ${JSON.stringify(bad)}`);
  }
});

test("a reader of a listed file, an inside-src-tauri file or a missing file passes the guard", () => {
  const ok = [
    'include_str!("../../../../src/bindings.ts")',
    'include_str!("../../../../src/locales/en/review.json")',
    'Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/locales/en").join("x.json")',
    'Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("scripts").join("preview.mjs")',
    'include_str!("../../Cargo.toml")',
    'include_str!("../tauri.conf.json")',
    'Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")',
    '"../Windows/System32"',
    'include_str!("../../../../src/no-such-file.ts")',
  ];
  for (const text of ok) assert.deepEqual(uncoveredReads([reader(text)]), [], text);
});

test("the guard goes by the real rules: take bindings.ts off the list and ipc.rs is reported", () => {
  const ipc = rustSources().find((s) => s.path === "src-tauri/src/ipc.rs");
  assert.ok(ipc);
  assert.deepEqual(uncoveredReads([ipc]), []);
  const bad = uncoveredReads([ipc], { needsRust: (p) => p !== "src/bindings.ts" });
  assert.deepEqual(bad.map((b) => b.path), ["src/bindings.ts"]);
});
