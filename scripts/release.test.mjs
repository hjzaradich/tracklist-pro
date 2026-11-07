// Tests for release.mjs. Run with: node --test scripts/release.test.mjs
// Everything is injected: no build, no network, no real git or gh.
import { test } from "node:test";
import assert from "node:assert/strict";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { bump, build, draft, main, VERSION_PLACES } from "./release.mjs";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
const real = { readFileSync, writeFileSync, existsSync, mkdirSync, copyFileSync };

/** A copy of the five real files in a temp folder. */
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "rel-"));
  for (const p of VERSION_PLACES) {
    mkdirSync(dirname(join(root, p.file)), { recursive: true });
    copyFileSync(join(REPO, p.file), join(root, p.file));
  }
  return root;
}
const ok = (stdout = "") => ({ status: 0, stdout, stderr: "" });

/** Injected side effects; `git` answers git calls by their first argument. */
function deps({ git = () => ok(), gh = (a) => (a[1] === "view" ? { status: 1, stdout: "", stderr: "not found" } : ok()) } = {}) {
  const logs = [];
  const calls = [];
  const d = {
    fs: real,
    log: (m) => logs.push(m),
    run: (cmd, args) => {
      calls.push([cmd, ...args]);
      return cmd === "git" ? git(args) : ok();
    },
    gh: (args) => {
      calls.push(["gh", ...args]);
      return gh(args);
    },
  };
  return { d, logs, calls };
}
const version = (root) => JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;

test("bump edits exactly the five places and nothing else", () => {
  const root = fixture();
  const before = VERSION_PLACES.map((p) => readFileSync(join(root, p.file), "utf8"));
  const cur = version(root);
  bump(root, "9.8.7", deps().d);
  let changed = 0;
  VERSION_PLACES.forEach((p, i) => {
    const a = before[i].split("\n");
    const b = readFileSync(join(root, p.file), "utf8").split("\n");
    assert.equal(a.length, b.length);
    const diff = a.map((l, n) => [l, b[n]]).filter(([x, y]) => x !== y);
    assert.equal(diff.length, p.count, p.file);
    for (const [x, y] of diff) assert.equal(x.replace(cur, "9.8.7"), y);
    changed += diff.length;
  });
  assert.equal(changed, 6);
});

test("bump refuses equal, lower and malformed versions", () => {
  const root = fixture();
  const cur = version(root);
  assert.throws(() => bump(root, cur, deps().d), /not greater/);
  assert.throws(() => bump(root, "0.0.1", deps().d), /not greater/);
  assert.throws(() => bump(root, "1.2", deps().d), /not a version/);
  assert.equal(version(root), cur);
});

test("build refuses a dirty tree", () => {
  const root = fixture();
  const { d, calls } = deps({ git: (a) => (a[0] === "status" ? ok(" M x") : ok()) });
  assert.throws(() => build(root, join(root, "out"), d), /not clean/);
  assert.ok(!calls.some((c) => c[0] === "npx"));
});

test("build refuses when HEAD is not at origin/main", () => {
  const root = fixture();
  const { d, calls } = deps({ git: (a) => (a[0] === "rev-parse" ? ok(a[1] === "HEAD" ? "aaa" : "bbb") : ok()) });
  assert.throws(() => build(root, join(root, "out"), d), /origin\/main/);
  assert.ok(!calls.some((c) => c[0] === "npx"));
});

test("build refuses to overwrite an existing output file", () => {
  const root = fixture();
  const out = join(root, "out");
  mkdirSync(out);
  writeFileSync(join(out, `tracklist-pro_${version(root)}_x64-setup.exe`), "old");
  const { d, calls } = deps();
  assert.throws(() => build(root, out, d), /already exists/);
  assert.equal(calls.length, 0);
});

test("build copies the installer and writes its sha256", () => {
  const root = fixture();
  const v = version(root);
  const nsis = join(root, "src-tauri", "target", "release", "bundle", "nsis");
  mkdirSync(nsis, { recursive: true });
  writeFileSync(join(nsis, `tracklist-pro_${v}_x64-setup.exe`), "abc");
  const { d, logs, calls } = deps({ git: (a) => (a[0] === "rev-parse" ? ok("same") : ok()) });
  const r = build(root, join(root, "out"), d);
  assert.deepEqual(calls.find((c) => c[0] === "npx"), ["npx", "tauri", "build", "--bundles", "nsis"]);
  const sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
  assert.equal(r.sha, sha);
  assert.match(readFileSync(`${r.dest}.sha256`, "utf8"), new RegExp(`^${sha}  tracklist-pro_`));
  assert.ok(logs.some((l) => l.includes(sha)) && logs.some((l) => l.includes("3 bytes")));
});

function draftSetup(opts) {
  const root = fixture();
  const v = version(root);
  const out = join(root, "out");
  mkdirSync(out);
  writeFileSync(join(out, `tracklist-pro_${v}_x64-setup.exe`), "abc");
  writeFileSync(join(out, `tracklist-pro_${v}_x64-setup.exe.sha256`), "deadbeef  x\n");
  return { root, out, v, ...deps(opts) };
}

test("draft creates a draft release and never publishes", () => {
  const s = draftSetup();
  draft(s.root, s.out, s.d);
  const create = s.calls.find((c) => c[0] === "gh" && c[1] === "release" && c[2] === "create");
  assert.ok(create.includes("--draft"));
  assert.ok(create.includes(`v${s.v}`) && create.includes("--notes-file"));
  assert.equal(create.filter((a) => a.endsWith(".exe") || a.endsWith(".sha256")).length, 2);
  const notes = readFileSync(join(s.out, `release-notes_${s.v}.md`), "utf8");
  assert.match(notes, /deadbeef/);
  assert.match(notes, /docs\/installing\.md/);
  assert.match(notes, /SmartScreen -> More info -> Run anyway/);
  for (const c of s.calls.filter((c) => c[0] === "gh")) assert.notEqual(c[2], "edit");
});

test("draft refuses an existing tag or release", () => {
  const local = draftSetup({ git: (a) => ok(a[0] === "tag" ? "v1" : "") });
  assert.throws(() => draft(local.root, local.out, local.d), /already exists locally/);
  const remote = draftSetup({ git: (a) => ok(a[0] === "ls-remote" ? "abc\trefs/tags/x" : "") });
  assert.throws(() => draft(remote.root, remote.out, remote.d), /on GitHub/);
  const rel = draftSetup({ gh: () => ok() });
  assert.throws(() => draft(rel.root, rel.out, rel.d), /release for/);
  assert.ok(!rel.calls.some((c) => c[2] === "create"));
});

test("there is no publish path", () => {
  const src = readFileSync(join(REPO, "scripts", "release.mjs"), "utf8");
  assert.ok(!/draft=false/.test(src));
  assert.ok(!/"edit"/.test(src));
  assert.ok(!/--latest|--prerelease/.test(src));
  assert.throws(() => main(["publish"], deps().d, REPO), /Usage/);
});
