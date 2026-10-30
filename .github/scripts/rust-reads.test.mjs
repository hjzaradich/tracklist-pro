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
//      Path::new("..") / PathBuf::from("..") may start the chain.
//   5. manifest_dir().parent() / env!("CARGO_MANIFEST_DIR").parent() followed
//      by .join("x") calls: relative to the repo root. When the root is kept
//      in a variable (no join straight after), every .join("name") in that
//      file that names a file at the repo root counts too.
// A backstop (nameReads) catches what those forms miss: any Rust string
// literal that equals the name of a top-level entry of src/ ("main.tsx",
// "locales") or a frontend root file ("index.html", "vite.config.ts",
// "package.json") counts as a read of it, wherever it appears, and must be
// covered too. A literal that only looks like such a name (a test's made-up
// file, an i18n key) is marked with the comment
//   // ci-reads: not a frontend read
// on its line or the line above.
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
  const rest = text.replace(/include_(?:str|bytes)!\(\s*"([^"]+)"\s*,?\s*\)/g, (_, rel) => {
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
  const dotdot = String.raw`(?:(?:Path::new|PathBuf::from)\(\s*"\.\."\s*\)|\.join\(\s*"\.\."\s*\))`;
  const chain = new RegExp(String.raw`((?:${dotdot}\s*)+)((?:\.join\(\s*"[^"]*"\s*\)\s*)+)`, "g");
  for (const m of rest.matchAll(chain)) {
    const ups = (m[1].match(/"\.\."/g) || []).length;
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

/** The names a Rust literal could be reading from the frontend: name -> repo-relative path. */
function frontendNames(root = repoRoot) {
  const names = new Map();
  for (const entry of readdirSync(join(root, "src"), { withFileTypes: true })) {
    names.set(entry.name, entry.isDirectory() ? `src/${entry.name}/` : `src/${entry.name}`);
  }
  const rootFile = /^(index\.html|(vite|vitest)\.config\.[cm]?[jt]s|tsconfig[^/]*\.json|eslint\.config\.[cm]?js|package(-lock)?\.json)$/;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.isFile() && rootFile.test(entry.name)) names.set(entry.name, entry.name);
  }
  return names;
}

const NOT_A_READ = /\/\/\s*ci-reads:\s*not a frontend read/;

/** A line without char literals and without a trailing // comment (outside a string). */
function code(line) {
  const l = line.replace(/'(?:\\.|[^'\\])'/g, "''");
  let out = "";
  let inString = false;
  for (let i = 0; i < l.length; i++) {
    const c = l[i];
    if (inString) {
      if (c === "\\") {
        out += c + (l[++i] ?? "");
        continue;
      }
      if (c === '"') inString = false;
    } else if (c === '"') {
      inString = true;
    } else if (c === "/" && l[i + 1] === "/") {
      break;
    }
    out += c;
  }
  return out;
}

/**
 * The backstop: string literals that equal a frontend name, unless marked.
 * Same shape as outsideReads, plus the line number.
 */
export function nameReads({ text }, root = repoRoot) {
  const names = frontendNames(root);
  const lines = text.split(/\r?\n/);
  const reads = [];
  lines.forEach((raw, i) => {
    if (NOT_A_READ.test(raw) || (i > 0 && NOT_A_READ.test(lines[i - 1]))) return;
    const src = code(raw);
    for (const m of src.matchAll(/"((?:[^"\\]|\\.)*)"/g)) {
      const path = names.get(m[1]);
      if (path === undefined) continue;
      // A folder's name is also an ordinary word in the domain ("scan",
      // "review", "send": job kinds, table names), so it counts only where
      // it is used as a path: the argument of a join, push or Path::new.
      if (path.endsWith("/")) {
        const before = src.slice(0, m.index).trimEnd();
        if (!/(?:\.join|\.push|Path::new|PathBuf::from)\($/.test(before)) continue;
      }
      reads.push({ literal: m[1], path, line: i + 1 });
    }
  });
  return reads;
}

/**
 * The reads that wouldn't run the Rust job when they change.
 * `needsRust(path)` says whether a change to `path` runs it.
 */
export function uncoveredReads(sources, { root = repoRoot, needsRust = (p) => jobsFor([p]).rust } = {}) {
  const bad = [];
  for (const source of sources) {
    for (const read of [...outsideReads(source, root), ...nameReads(source, root)]) {
      // A folder is read whole: a new file in it must run the job too.
      const probe = read.path.endsWith("/") ? `${read.path}__new_file__` : read.path;
      if (!needsRust(probe)) bad.push({ file: source.path, ...read });
    }
  }
  return bad;
}

const describe = (bad) =>
  bad
    .map(
      (b) =>
        `${b.file}${b.line ? `:${b.line}` : ""} reads ${b.path} (${b.literal}); add it to RUST_READS (frontend files) or SHARED in ci-changes.mjs, or if the literal is not a read, mark its line (or the one above) with // ci-reads: not a frontend read`,
    )
    .join("\n");

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
  assert.ok(isFrontendOnly("src/vite-env.d.ts"));
  assert.ok(isFrontendOnly("src/locales/notes/review.json"));
  assert.ok(!jobsFor(["src/vite-env.d.ts"]).rust);
});

// Proof that the guard fails for an unlisted reader.

const reader = (text, path = "src-tauri/src/review/tests/x.rs") => ({ path, text });

test("a new Rust test reading an unlisted frontend file fails the guard", () => {
  // src/theme/themeStore.ts and src/theme exist and are frontend-only: nothing in
  // RUST_READS covers them.
  const cases = {
    "include_str!": 'let s = include_str!("../../../../src/theme/themeStore.ts");',
    "include_bytes!": 'let s = include_bytes!("../../../../src/theme/themeStore.ts");',
    "CARGO_MANIFEST_DIR concat": 'let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/theme/themeStore.ts");',
    "a ../ literal": 'let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/theme/themeStore.ts");',
    "a .join(..) chain": 'let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("src/theme/themeStore.ts");',
    "a folder": 'let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/theme");',
    "the repo root's child": 'let p = manifest_dir().parent().unwrap().join("src/theme/themeStore.ts");',
    "a root variable": 'let root = manifest_dir().parent().unwrap().to_path_buf();\nlet p = root.join("index.html");',
    // The forms the first version missed.
    "include_str! split over lines, with a trailing comma":
      'let s = include_str!(\n    "../../../../src/theme/themeStore.ts",\n);',
    "include_bytes! split over lines": 'let s = include_bytes!(\n    "../../../../src/theme/themeStore.ts"\n);',
    "a Path::new(..) chain": 'let p = Path::new("..").join("src/theme/themeStore.ts");',
    "a PathBuf::from(..) chain split over lines": 'let p = PathBuf::from("..")\n    .join("src/theme")\n    .join("themeStore.ts");',
    // The backstop, alone: no path form reaches these.
    "the backstop: a frontend file's name": 'let name = "vite-env.d.ts";',
    "the backstop: a frontend folder's name used as a path": 'let dir = base.join("theme");',
    "the backstop: a frontend root file's name": 'let name = "index.html";',
  };
  for (const [name, text] of Object.entries(cases)) {
    const bad = uncoveredReads([reader(text)]);
    assert.ok(bad.length >= 1, `${name}: expected an uncovered read, got ${JSON.stringify(bad)}`);
  }
});

test("the backstop is quiet for a marked line, a covered name, a comment and a plain word", () => {
  const ok = [
    'let name = "vite-env.d.ts"; // ci-reads: not a frontend read',
    '// ci-reads: not a frontend read\nlet name = "vite-env.d.ts";',
    'let name = "main.tsx";',
    'let name = "bindings.ts";',
    '// "vite-env.d.ts" is mentioned in a comment',
    'let kind = "scan"; let table = "review";',
    'let p = tmp.join("scan"); // ci-reads: not a frontend read',
  ];
  for (const text of ok) assert.deepEqual(uncoveredReads([reader(text)]), [], text);
  // The marker covers its own line and the next one only.
  assert.equal(uncoveredReads([reader('// ci-reads: not a frontend read\nlet a = 1;\nlet n = "vite-env.d.ts";')]).length, 1);
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
  assert.deepEqual([...new Set(bad.map((b) => b.path))], ["src/bindings.ts"]);
});
