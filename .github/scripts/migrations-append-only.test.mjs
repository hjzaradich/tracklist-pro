// Tests for migrations-append-only.mjs. Run with: node --test .github/scripts/
// The check() tests build throwaway git repos, so they need git on PATH.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, renameSync, appendFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sqlProblems, checksumProblems, check, CHECKSUMS } from "./migrations-append-only.mjs";

test("sqlProblems allows added .sql files and flags every other change", () => {
  const out = [
    "A", "src-tauri/migrations/0002_new.sql",
    "M", "src-tauri/migrations/0001_init.sql",
    "D", "src-tauri/migrations/0003_gone.sql",
    "T", "src-tauri/migrations/0004_link.sql",
    "M", "src-tauri/migrations/README.md",
    "M", "src-tauri/migrations/checksums.txt",
    "M", "src-tauri/src/db/0001_init.sql",
    "",
  ].join("\0");
  assert.deepEqual(sqlProblems(out), [
    "src-tauri/migrations/0001_init.sql: modified; migrations on the base branch are append-only",
    "src-tauri/migrations/0003_gone.sql: deleted or renamed; migrations on the base branch are append-only",
    "src-tauri/migrations/0004_link.sql: changed type; migrations on the base branch are append-only",
  ]);
  assert.deepEqual(sqlProblems(""), []);
});

test("checksumProblems flags removed and changed lines, not added, comment or blank ones", () => {
  const diff = `diff --git a/${CHECKSUMS} b/${CHECKSUMS}
index 1111111..2222222 100644
--- a/${CHECKSUMS}
+++ b/${CHECKSUMS}
@@ -1 +1 @@
-# old comment
+# new comment
@@ -4 +4 @@
-0001_init.sql aaaa
+0001_init.sql bbbb
@@ -6,0 +7,2 @@
+0003_c.sql cccc
+
@@ -8,2 +9,0 @@
-0002_b.sql dddd
-
`;
  assert.deepEqual(checksumProblems(diff), [
    `${CHECKSUMS}: line changed or removed: 0001_init.sql aaaa`,
    `${CHECKSUMS}: line changed or removed: 0002_b.sql dddd`,
  ]);
  // A removed line whose text starts with "--" isn't mistaken for a header.
  assert.equal(checksumProblems("@@ -1 +0,0 @@\n--- odd.sql eeee\n").length, 1);
  assert.deepEqual(checksumProblems(""), []);
});

// check() against real git history.

function repo() {
  const dir = mkdtempSync(join(tmpdir(), "migrations-append-only-"));
  const run = (...args) => execFileSync("git", args, { cwd: dir, encoding: "utf8" });
  run("init", "-q", "-b", "main");
  run("config", "user.name", "test");
  run("config", "user.email", "test@example.invalid");
  run("config", "core.autocrlf", "false");
  const commit = (msg) => {
    run("add", "-A");
    run("commit", "-q", "--allow-empty", "-m", msg);
  };
  const file = (rel, text) => {
    const path = join(dir, rel);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, text);
  };
  return { dir, run, commit, file, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

const HEADER = "# Append-only record.\n";

/** A base branch with one migration, then a PR branch `pr` off it. */
function withBase() {
  const r = repo();
  r.file("README.md", "x\n");
  r.file("src-tauri/migrations/0001_init.sql", "CREATE TABLE t (id INTEGER);\n");
  r.file(CHECKSUMS, `${HEADER}0001_init.sql aaaa\n`);
  r.commit("base");
  r.run("switch", "-q", "-c", "pr");
  return r;
}

test("check passes when the base has no migrations folder, and when it gains one", (t) => {
  const r = repo();
  t.after(r.cleanup);
  r.file("README.md", "x\n");
  r.commit("base");
  r.run("switch", "-q", "-c", "pr");
  assert.deepEqual(check("main", r.dir), []);
  r.file("src-tauri/migrations/0001_init.sql", "CREATE TABLE t (id INTEGER);\n");
  r.file(CHECKSUMS, `${HEADER}0001_init.sql aaaa\n`);
  r.commit("add migrations");
  assert.deepEqual(check("main", r.dir), []);
});

test("check passes a PR that only appends a migration and its checksum line", (t) => {
  const r = withBase();
  t.after(r.cleanup);
  r.file("src-tauri/migrations/0002_next.sql", "CREATE TABLE u (id INTEGER);\n");
  appendFileSync(join(r.dir, CHECKSUMS), "0002_next.sql bbbb\n");
  r.commit("append");
  assert.deepEqual(check("main", r.dir), []);
});

test("check fails a PR that edits a migration and its checksum line together", (t) => {
  const r = withBase();
  t.after(r.cleanup);
  r.file("src-tauri/migrations/0001_init.sql", "CREATE TABLE t (id INTEGER, x TEXT);\n");
  r.file(CHECKSUMS, `${HEADER}0001_init.sql ffff\n`);
  r.commit("rewrite history");
  assert.deepEqual(check("main", r.dir), [
    "src-tauri/migrations/0001_init.sql: modified; migrations on the base branch are append-only",
    `${CHECKSUMS}: line changed or removed: 0001_init.sql aaaa`,
  ]);
});

test("check fails a renamed or deleted migration and a deleted checksums file", (t) => {
  const r = withBase();
  t.after(r.cleanup);
  renameSync(join(r.dir, "src-tauri/migrations/0001_init.sql"), join(r.dir, "src-tauri/migrations/0001_first.sql"));
  rmSync(join(r.dir, CHECKSUMS));
  r.commit("rename and delete");
  const problems = check("main", r.dir);
  assert.equal(problems.length, 2, problems.join("\n"));
  assert.match(problems[0], /0001_init\.sql: deleted or renamed/);
  assert.match(problems[1], /line changed or removed: 0001_init\.sql aaaa/);
});

test("check compares with the merge base, so later base commits don't count", (t) => {
  const r = withBase();
  t.after(r.cleanup);
  r.file("feature.txt", "y\n");
  r.commit("unrelated PR work");
  // main moves on after the PR branched, and edits its own history (as if
  // someone had force-pushed); the PR itself is still clean.
  r.run("switch", "-q", "main");
  r.file("src-tauri/migrations/0001_init.sql", "-- changed on main\n");
  r.commit("main moves");
  r.run("switch", "-q", "pr");
  assert.deepEqual(check("main", r.dir), []);
});

test("check only accepts comment and blank line edits in checksums.txt", (t) => {
  const r = withBase();
  t.after(r.cleanup);
  r.file(CHECKSUMS, `# Reworded header.\n\n0001_init.sql aaaa\n`);
  r.commit("reword comment");
  assert.deepEqual(check("main", r.dir), []);
});
