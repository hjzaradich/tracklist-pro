// Tests for ci-changes.mjs. Run with: node --test .github/scripts/
// The changedFiles() tests build throwaway git repos, so they need git on PATH.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { areas, jobsFor, changesCacheKeys, decide, changedFiles, markerName, parseMarker, covers, SHARED, resultProblems, AREA_JOBS, JOBS } from "./ci-changes.mjs";

const none = { app: false, fixture_gen: false, rb_kit: false };
const every = { app: true, fixture_gen: true, rb_kit: true };

const area = (path) => areas(path).join("+");

test("each path lands in the area whose checks it can affect", () => {
  assert.equal(area(".github/workflows/ci-windows.yml"), "ci");
  assert.equal(area(".github/scripts/npm-licenses.mjs"), "ci");
  assert.equal(area("README.md"), "docs");
  assert.equal(area("TASKS.md"), "docs");
  assert.equal(area("spikes/rb-kit/CHECK.md"), "docs");
  assert.equal(area("docs/Notes.MD"), "docs");
  assert.equal(area("tools/fixture-gen/src/plan.rs"), "fixture_gen");
  assert.equal(area("tools/fixture-gen/Cargo.lock"), "fixture_gen");
  assert.equal(area("spikes/rb-kit/rb_check.py"), "rb_kit");
  assert.equal(area("spikes/audio-lab/main.py"), "spike");
  assert.equal(area("src-tauri/src/tags/key.rs"), "app");
  assert.equal(area("src/App.tsx"), "app");
  assert.equal(area("package-lock.json"), "app");
  assert.equal(area("deny.toml"), "app");
  // Only a real .md name is docs; a file that merely mentions "md" is not.
  assert.equal(area("src/markdown.ts"), "app");
});

test("only the root LICENSE file is docs, not LICENSE files elsewhere or LICENSE.*", () => {
  assert.equal(area("LICENSE"), "docs");
  assert.equal(area("LICENSE.txt"), "app");
  assert.equal(area("src-tauri/LICENSE"), "app");
  assert.equal(area("src-tauri/src/LICENSE_NOTES.rs"), "app");
  assert.equal(area("tools/fixture-gen/LICENSE"), "fixture_gen");
});

test(".gitattributes changes every checkout, so it counts as a CI change and runs everything", () => {
  assert.equal(area(".gitattributes"), "ci");
  assert.deepEqual(jobsFor([".gitattributes"]), every);
});

test("the app's Cargo.toml and Cargo.lock also run the fixture generator, whose dev-only test reads them", () => {
  assert.equal(area("src-tauri/Cargo.toml"), "app+fixture_gen");
  assert.equal(area("src-tauri/Cargo.lock"), "app+fixture_gen");
  assert.deepEqual(jobsFor(["src-tauri/Cargo.lock"]), { ...none, app: true, fixture_gen: true });
  // Other app files don't.
  assert.deepEqual(jobsFor(["src-tauri/src/lib.rs"]), { ...none, app: true });
});

test("every cross-area file listed in SHARED is really read by the other area's test", () => {
  // Keeps SHARED honest: the reader's source names each listed file.
  const readers = { fixture_gen: "tools/fixture-gen/tests/dev_only.rs" };
  for (const [path, owners] of Object.entries(SHARED)) {
    for (const owner of owners.filter((o) => o in readers)) {
      const src = readFileSync(new URL(`../../${readers[owner]}`, import.meta.url), "utf8");
      assert.ok(src.includes(path), `${readers[owner]} doesn't read ${path}`);
    }
  }
});

test("a docs-only change runs no build or test job", () => {
  assert.deepEqual(jobsFor(["README.md", "ROADMAP.md", "spikes/rb-kit/SESSION.md"]), none);
});

test("a change to CI itself runs every job", () => {
  assert.deepEqual(jobsFor(["README.md", ".github/workflows/ci-windows.yml"]), every);
});

test("a fixture generator change runs only its job", () => {
  assert.deepEqual(jobsFor(["tools/fixture-gen/src/plan.rs"]), { ...none, fixture_gen: true });
});

test("a check kit change runs only its job", () => {
  assert.deepEqual(jobsFor(["spikes/rb-kit/rb_check.py"]), { ...none, rb_kit: true });
});

test("a change to other spikes runs nothing, since CI builds no other spike", () => {
  assert.deepEqual(jobsFor(["spikes/audio-lab/x.py", "spikes/inventory.py"]), none);
});

test("an app change runs the app job, and mixed changes run each area's job", () => {
  assert.deepEqual(jobsFor(["src-tauri/src/lib.rs"]), { ...none, app: true });
  assert.deepEqual(jobsFor(["src/main.tsx", "spikes/rb-kit/rbxml.py", "TASKS.md"]), { ...none, app: true, rb_kit: true });
});

test("an empty change runs nothing", () => {
  assert.deepEqual(jobsFor([]), none);
});

test("manifests, lockfiles (Cargo and npm), the toolchain and workflows change cache keys; code does not", () => {
  assert.ok(changesCacheKeys(["src-tauri/Cargo.lock"]));
  assert.ok(changesCacheKeys(["src-tauri/Cargo.toml"]));
  assert.ok(changesCacheKeys(["tools/fixture-gen/Cargo.toml"]));
  assert.ok(changesCacheKeys(["rust-toolchain.toml"]));
  assert.ok(changesCacheKeys([".github/workflows/ci-windows.yml"]));
  assert.ok(changesCacheKeys(["package-lock.json"]));
  assert.ok(!changesCacheKeys(["src-tauri/src/lib.rs", "src/App.tsx", ".github/scripts/ci-changes.mjs"]));
  assert.ok(!changesCacheKeys(["src-tauri/src/Cargo.lock.rs"]));
});

test("scheduled and manual runs run everything, whatever changed", () => {
  assert.deepEqual(decide({ event: "schedule", files: null }).jobs, every);
  assert.deepEqual(decide({ event: "workflow_dispatch", files: ["README.md"] }).jobs, every);
});

test("a run whose changed files can't be listed runs everything", () => {
  assert.deepEqual(decide({ event: "push", files: null }).jobs, every);
  assert.deepEqual(decide({ event: "pull_request", files: null }).jobs, every);
});

test("a docs-only pull request runs nothing but still gets a reason", () => {
  const d = decide({ event: "pull_request", files: ["TASKS.md"] });
  assert.deepEqual(d.jobs, none);
  assert.match(d.reason, /nothing runs/);
});

test("a push to main whose exact tree already passed on its pull request runs nothing again", () => {
  const d = decide({ event: "push", files: ["src-tauri/src/lib.rs", "src/App.tsx"], tested: [{ ...none, app: true }] });
  assert.deepEqual(d.jobs, none);
  assert.match(d.reason, /already tested this exact tree/);
});

test("a tested push that changes cache keys still runs, so main's caches are rebuilt", () => {
  const d = decide({ event: "push", files: ["src-tauri/Cargo.lock"], tested: [every] });
  assert.deepEqual(d.jobs, { ...none, app: true, fixture_gen: true });
  const npm = decide({ event: "push", files: ["package-lock.json"], tested: [{ ...none, app: true }] });
  assert.deepEqual(npm.jobs, { ...none, app: true });
  const ci = decide({ event: "push", files: [".github/workflows/ci-windows.yml"], tested: [every] });
  assert.deepEqual(ci.jobs, every);
});

test("a push to main whose tree no pull request run tested runs the jobs its changes need", () => {
  assert.deepEqual(decide({ event: "push", files: ["src/App.tsx"], tested: [] }).jobs, { ...none, app: true });
  assert.deepEqual(decide({ event: "push", files: ["tools/fixture-gen/src/lib.rs"] }).jobs, { ...none, fixture_gen: true });
});

test("a pull request ignores tested-tree markers: its own run is the test", () => {
  const d = decide({ event: "pull_request", files: ["src/App.tsx"], tested: [{ ...none, app: true }] });
  assert.deepEqual(d.jobs, { ...none, app: true });
});

test("a tested push skips only if the PR run covered every job the push's own diff needs", () => {
  // The same tree reached by another route: the marking run was a docs-only
  // PR (it ran nothing), but this push's diff touches the app.
  const docsOnlyRun = none;
  const d = decide({ event: "push", files: ["src/App.tsx"], tested: [docsOnlyRun] });
  assert.deepEqual(d.jobs, { ...none, app: true });
  // A run that ran the app, but not the check kit the push also needs.
  const partial = decide({ event: "push", files: ["src/App.tsx", "spikes/rb-kit/rbxml.py"], tested: [{ ...none, app: true }] });
  assert.deepEqual(partial.jobs, { ...none, app: true, rb_kit: true });
  // Any one covering run is enough.
  const covered = decide({ event: "push", files: ["src/App.tsx"], tested: [docsOnlyRun, every] });
  assert.deepEqual(covered.jobs, none);
});

test("covers is true only when every needed job ran", () => {
  assert.ok(covers(every, { ...none, app: true }));
  assert.ok(covers(none, none));
  assert.ok(!covers({ ...none, app: true }, { ...none, fixture_gen: true }));
});

test("a marker names the tree and exactly the jobs that ran, and reads back the same", () => {
  const tree = "0123456789abcdef0123456789abcdef01234567";
  assert.equal(markerName(tree, none), `ci-passed-tree-${tree}-ran-none`);
  assert.equal(markerName(tree, { ...none, app: true, rb_kit: true }), `ci-passed-tree-${tree}-ran-app.rb_kit`);
  for (const jobs of [none, every, { ...none, fixture_gen: true }, { ...none, app: true, rb_kit: true }]) {
    assert.deepEqual(parseMarker(markerName(tree, jobs), tree), jobs);
  }
});

test("a marker for another tree, or naming an unknown job, is ignored", () => {
  const tree = "0123456789abcdef0123456789abcdef01234567";
  const other = "fedcba9876543210fedcba9876543210fedcba98";
  assert.equal(parseMarker(markerName(other, every), tree), null);
  assert.equal(parseMarker(`ci-passed-tree-${tree}-ran-app.mystery`, tree), null);
  assert.equal(parseMarker(`ci-passed-tree-${tree}`, tree), null);
  assert.equal(parseMarker("", tree), null);
});

// resultProblems(): the CI result job's verdict.

const ok = { result: "success" };
const skipped = { result: "skipped" };
function needs(flags, results = {}) {
  const outputs = { app: "false", fixture_gen: "false", rb_kit: "false", marker: "m", ...flags };
  return {
    changes: { result: "success", outputs },
    windows: skipped, "fixture-gen": skipped, "rb-kit": skipped, checks: skipped, licenses: skipped,
    ...results,
  };
}

test("a run passes when every needed job passed and the rest were skipped", () => {
  assert.deepEqual(resultProblems(needs({ app: "true" }, { windows: ok, checks: ok, licenses: ok })), []);
  assert.deepEqual(resultProblems(needs({ rb_kit: "true" }, { "rb-kit": ok })), []);
  // Docs only: nothing needed, everything skipped.
  assert.deepEqual(resultProblems(needs({})), []);
});

test("a run fails when any job failed or was cancelled", () => {
  assert.deepEqual(resultProblems(needs({ app: "true" }, { windows: { result: "failure" }, checks: ok, licenses: ok })), [
    "windows: failure",
    "windows: needed for app, but its result is failure",
  ]);
  assert.deepEqual(resultProblems(needs({}, { licenses: { result: "cancelled" } })), ["licenses: cancelled"]);
  assert.deepEqual(resultProblems({ changes: { result: "failure", outputs: {} } }), [
    "changes: failure",
    "changes: failure; the gate must pass",
  ]);
});

test("a run fails, never reports green, when the gate job was skipped, failed or is missing", () => {
  // Skipped: every other job is skipped too, which alone would look green.
  const skippedGate = { ...needs({}), changes: { result: "skipped", outputs: {} } };
  assert.deepEqual(resultProblems(skippedGate), ["changes: skipped; the gate must pass"]);
  const failedGate = { ...needs({}), changes: { result: "failure", outputs: {} } };
  assert.ok(resultProblems(failedGate).includes("changes: failure; the gate must pass"));
  const { changes, ...noGate } = needs({});
  assert.ok(changes);
  assert.deepEqual(resultProblems(noGate), ["changes: missing; the gate must pass"]);
});

test("a run fails when a job its changes needed was skipped, so its marker can't claim that job", () => {
  assert.deepEqual(resultProblems(needs({ app: "true" }, { checks: ok, licenses: ok })), [
    "windows: needed for app, but its result is skipped",
  ]);
  assert.deepEqual(resultProblems(needs({ fixture_gen: "true" }, { "fixture-gen": ok, checks: ok })), [
    "licenses: needed for fixture_gen, but its result is skipped",
  ]);
});

// The workflow itself, read from ci-windows.yml, so the script and the
// workflow can't drift apart. A small reader, enough for this file's layout:
// job ids at two spaces under `jobs:`, their `needs:` and `if:` at four.

function workflowJobs() {
  const yml = readFileSync(new URL("../workflows/ci-windows.yml", import.meta.url), "utf8");
  const lines = yml.split(/\r?\n/);
  const jobs = {};
  let current = null;
  for (const line of lines.slice(lines.indexOf("jobs:") + 1)) {
    const id = line.match(/^ {2}([A-Za-z0-9_-]+):\s*$/);
    if (id) {
      current = jobs[id[1]] = { needs: [], if: "" };
      continue;
    }
    if (/^\S/.test(line)) break; // the end of the jobs: block
    const needsLine = line.match(/^ {4}needs:\s*(.*)$/);
    if (current && needsLine) {
      current.needs = needsLine[1].replace(/^\[|\]$/g, "").split(",").map((j) => j.trim()).filter(Boolean);
    }
    const ifLine = line.match(/^ {4}if:\s*(.*)$/);
    if (current && ifLine) current.if = ifLine[1];
  }
  return jobs;
}

test("CI result waits for every other job the workflow has", () => {
  const jobs = workflowJobs();
  const ids = Object.keys(jobs);
  // Guards the reader itself: it must find the jobs, not an empty list.
  assert.ok(ids.includes("changes") && ids.includes("windows") && ids.includes("result"), ids.join(", "));
  const others = ids.filter((id) => id !== "result").sort();
  assert.deepEqual([...jobs.result.needs].sort(), others);
});

test("each job's if: gating in the workflow matches the areas that list it in AREA_JOBS", () => {
  const jobs = workflowJobs();
  for (const [id, job] of Object.entries(jobs)) {
    if (id === "changes" || id === "result") continue;
    const gatedBy = [...job.if.matchAll(/needs\.changes\.outputs\.(\w+) == 'true'/g)].map((m) => m[1]).sort();
    const listedIn = JOBS.filter((area) => AREA_JOBS[area].includes(id)).sort();
    assert.ok(gatedBy.length > 0, `${id} isn't gated by the changes job`);
    assert.deepEqual(gatedBy, listedIn, `${id}: if: says ${gatedBy}, AREA_JOBS says ${listedIn}`);
  }
  // And nothing in AREA_JOBS names a job the workflow doesn't have.
  for (const [area, ids] of Object.entries(AREA_JOBS)) {
    for (const id of ids) assert.ok(id in jobs, `AREA_JOBS.${area} names ${id}, which isn't a job`);
  }
});

// changedFiles() against real git history.

function repo() {
  const dir = mkdtempSync(join(tmpdir(), "ci-changes-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, encoding: "utf8" }).trim();
  git("init", "-q", "-b", "main");
  git("config", "user.email", "ci@example.invalid");
  git("config", "user.name", "CI test");
  git("config", "commit.gpgsign", "false");
  const commit = (files, msg) => {
    for (const [path, body] of Object.entries(files)) {
      mkdirSync(join(dir, path, ".."), { recursive: true });
      writeFileSync(join(dir, path), body);
    }
    git("add", "-A");
    git("commit", "-q", "-m", msg);
    return git("rev-parse", "HEAD");
  };
  return { dir, git, commit, done: () => rmSync(dir, { recursive: true, force: true }) };
}

test("changedFiles lists the paths changed between two commits, odd names included", () => {
  const r = repo();
  const cwd = process.cwd();
  try {
    const first = r.commit({ "README.md": "a", "src/App.tsx": "a" }, "one");
    r.commit({ "src/App.tsx": "b", "spikes/rb-kit/Café #1 [x].py": "c" }, "two");
    process.chdir(r.dir);
    assert.deepEqual(changedFiles(first).sort(), ["spikes/rb-kit/Café #1 [x].py", "src/App.tsx"]);
    assert.deepEqual(changedFiles("HEAD^1"), changedFiles(first));
  } finally {
    process.chdir(cwd);
    r.done();
  }
});

test("changedFiles returns null for a missing, unknown or all-zero base, so everything runs", () => {
  const r = repo();
  const cwd = process.cwd();
  try {
    r.commit({ "a.txt": "a" }, "one");
    process.chdir(r.dir);
    assert.equal(changedFiles(undefined), null);
    assert.equal(changedFiles("0000000000000000000000000000000000000000"), null);
    assert.equal(changedFiles("1234567890abcdef1234567890abcdef12345678"), null);
  } finally {
    process.chdir(cwd);
    r.done();
  }
});
