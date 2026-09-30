// Decides which CI (Windows) jobs a run needs, so a change pays only for the
// checks it can affect (0E-13). Each changed path falls in one or more areas:
//   ci           .github/**, .gitattributes   everything runs: CI itself changed
//   docs         *.md, the root LICENSE        nothing to build or test
//   fixture_gen  tools/fixture-gen/**          the fixture generator's job
//   rb_kit       spikes/rb-kit/**              the rekordbox check kit's job
//   spike        other spikes/**               nothing: CI builds no other spike
//   app          anything else                 the app's build, tests and lints
// A file one area's tests read from another area's folder belongs to both
// (SHARED below). Scheduled and manual runs run everything. So does a push
// whose diff can't be read (a new branch, or a force push that dropped the
// old head).
//
// A push to main can skip everything: when a pull request run into main
// already passed on the very same tree (a squash merge onto an unchanged main
// gives exactly the tree that PR run tested), testing it again proves
// nothing new. A passing PR run leaves a marker artifact named for the tree
// and the jobs it ran (markerName); the push skips only if one of those runs
// covered every job the push's own diff needs. The push still runs when it
// changes what the caches are keyed on (Cargo manifests, lockfiles, the
// toolchain, package-lock.json or the workflows): only main's runs save
// caches, so skipping those would leave every later PR run cold.
//
// Usage (in CI): node .github/scripts/ci-changes.mjs
//   env EVENT           github.event_name
//       BEFORE          push only: the commit main was at before the push
//       MARKERS_FILE    push only: names of the marker artifacts found for
//                       this tree, one per line
// Writes app, fixture_gen and rb_kit (each "true" or "false"), and marker
// (this run's marker name), to $GITHUB_OUTPUT, and says why in the log and
// the job summary.
//
//        node .github/scripts/ci-changes.mjs result
//   env NEEDS  the CI result job's toJSON(needs)
// Exits 1 and lists the problems if the run didn't pass (resultProblems).

import { execFileSync } from "node:child_process";
import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const JOBS = ["app", "fixture_gen", "rb_kit"];

const all = (value) => Object.fromEntries(JOBS.map((job) => [job, value]));

// Files one area's tests read from outside its folder, e.g. with
// include_str!, so a change to them must run each reader's job too. Every
// cross-area read must be listed here, or a change to the file skips the
// test that reads it.
export const SHARED = {
  // tools/fixture-gen/tests/dev_only.rs: the app never depends on the dev tool.
  "src-tauri/Cargo.toml": ["app", "fixture_gen"],
  "src-tauri/Cargo.lock": ["app", "fixture_gen"],
};

/** The areas a changed path belongs to (see the table at the top). */
export function areas(path) {
  if (path.startsWith(".github/") || path === ".gitattributes") return ["ci"];
  if (path in SHARED) return SHARED[path];
  if (/\.md$/i.test(path) || path === "LICENSE") return ["docs"];
  if (path.startsWith("tools/fixture-gen/")) return ["fixture_gen"];
  if (path.startsWith("spikes/rb-kit/")) return ["rb_kit"];
  if (path.startsWith("spikes/")) return ["spike"];
  return ["app"];
}

/** Which jobs a set of changed paths needs. */
export function jobsFor(files) {
  const jobs = all(false);
  for (const file of files) {
    for (const a of areas(file)) {
      if (a === "ci") return all(true);
      if (a in jobs) jobs[a] = true;
    }
  }
  return jobs;
}

/** True if a change touches what the build caches are keyed on, or the workflows that save them. */
export function changesCacheKeys(files) {
  return files.some(
    (f) =>
      /(^|\/)Cargo\.(toml|lock)$/.test(f) ||
      /(^|\/)rust-toolchain(\.toml)?$/.test(f) ||
      /(^|\/)package-lock\.json$/.test(f) ||
      f.startsWith(".github/workflows/"),
  );
}

export const MARKER_PREFIX = "ci-passed-tree-";

/** The marker a passing run leaves: the tree it tested and the jobs it ran. */
export function markerName(tree, jobs) {
  const ran = JOBS.filter((j) => jobs[j]);
  return `${MARKER_PREFIX}${tree}-ran-${ran.length ? ran.join(".") : "none"}`;
}

/** The jobs a marker for `tree` says ran, or null if it's not a marker for that tree. */
export function parseMarker(name, tree) {
  const prefix = `${MARKER_PREFIX}${tree}-ran-`;
  if (!name.startsWith(prefix)) return null;
  const ran = name.slice(prefix.length);
  if (ran === "none") return all(false);
  const names = ran.split(".");
  if (!names.every((n) => JOBS.includes(n))) return null;
  return Object.fromEntries(JOBS.map((j) => [j, names.includes(j)]));
}

/** True if a run that ran `ran` covered every job in `needed`. */
export function covers(ran, needed) {
  return JOBS.every((j) => !needed[j] || ran[j]);
}

/**
 * Returns { jobs, reason }.
 *   event        github.event_name
 *   files        changed paths, or null if the diff couldn't be read
 *   tested       push only: for each passing PR run of this exact tree, the
 *                jobs it ran (from its marker)
 */
export function decide({ event, files, tested = [] }) {
  if (event === "schedule" || event === "workflow_dispatch") {
    return { jobs: all(true), reason: `${event} run: everything runs` };
  }
  if (files === null) {
    return { jobs: all(true), reason: "the changed files can't be listed: everything runs" };
  }
  const jobs = jobsFor(files);
  const running = JOBS.filter((j) => jobs[j]);
  const needs = running.length ? `changed areas need: ${running.join(", ")}` : "only docs or unbuilt spikes changed: nothing runs";
  if (event === "push" && tested.some((ran) => covers(ran, jobs))) {
    if (!changesCacheKeys(files)) {
      return {
        jobs: all(false),
        reason: "a passing pull request run already tested this exact tree with every job it needs: nothing runs again",
      };
    }
    return {
      jobs,
      reason: `a pull request run tested this tree, but the push changes cache keys, so main rebuilds its caches; ${needs}`,
    };
  }
  return { jobs, reason: needs };
}

/** The jobs (workflow job ids) each area needs; mirrors the job conditions in ci-windows.yml. */
export const AREA_JOBS = {
  app: ["windows", "checks", "licenses"],
  fixture_gen: ["fixture-gen", "checks", "licenses"],
  rb_kit: ["rb-kit"],
};

/**
 * Why a run failed, from the workflow's `needs` context (the CI result
 * job's toJSON(needs)); empty if it passed. A run fails if any job failed
 * or was cancelled, or if a job the changes needed didn't pass (e.g. was
 * skipped), so a passing run's marker never claims a job that didn't pass.
 */
export function resultProblems(needs) {
  const problems = [];
  for (const [job, { result }] of Object.entries(needs)) {
    if (result !== "success" && result !== "skipped") problems.push(`${job}: ${result}`);
  }
  // The gate decides what runs; if it didn't succeed, nothing it skipped was
  // proven unneeded, so the run fails whatever the other jobs did.
  const changes = needs.changes?.result ?? "missing";
  if (changes !== "success") problems.push(`changes: ${changes}; the gate must pass`);
  const outputs = needs.changes?.outputs;
  if (changes === "success" && !outputs) problems.push("changes: no outputs");
  for (const area of JOBS) {
    if (outputs?.[area] !== "true") continue;
    for (const job of AREA_JOBS[area]) {
      const result = needs[job]?.result ?? "missing";
      if (result !== "success") problems.push(`${job}: needed for ${area}, but its result is ${result}`);
    }
  }
  return [...new Set(problems)];
}

function git(args) {
  return execFileSync("git", args, { encoding: "utf8" });
}

/** Changed paths between two commits, or null if either is missing. */
export function changedFiles(from, to = "HEAD") {
  if (!from || /^0+$/.test(from)) return null;
  try {
    git(["cat-file", "-e", `${from}^{commit}`]);
  } catch {
    return null;
  }
  return git(["diff", "--name-only", "--no-renames", "-z", from, to]).split("\0").filter(Boolean);
}

function checkResult() {
  const needs = JSON.parse(process.env.NEEDS);
  for (const [job, { result }] of Object.entries(needs)) console.log(`${job}: ${result}`);
  const problems = resultProblems(needs);
  if (problems.length > 0) {
    for (const p of problems) console.error(`::error::${p}`);
    process.exit(1);
  }
  console.log("Every job passed, or was skipped because the changes didn't need it.");
}

function main() {
  if (process.argv[2] === "result") return checkResult();
  const event = process.env.EVENT;
  const tree = git(["rev-parse", "HEAD^{tree}"]).trim();
  let files = null;
  // A pull_request run checks out GitHub's merge of the PR into its base;
  // the merge's first parent is the base, so this is exactly the PR's change.
  if (event === "pull_request") files = changedFiles("HEAD^1");
  if (event === "push") files = changedFiles(process.env.BEFORE);
  const markersFile = process.env.MARKERS_FILE;
  const tested =
    markersFile && existsSync(markersFile)
      ? readFileSync(markersFile, "utf8")
          .split("\n")
          .map((name) => parseMarker(name.trim(), tree))
          .filter(Boolean)
      : [];
  const { jobs, reason } = decide({ event, files, tested });
  const marker = markerName(tree, jobs);

  const lines = [`Changed paths: ${reason}.`, ...JOBS.map((j) => `  ${j}: ${jobs[j]}`), `  marker: ${marker}`];
  console.log(lines.join("\n"));
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `${JOBS.map((j) => `${j}=${jobs[j]}\n`).join("")}marker=${marker}\n`);
  }
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${lines.join("\n")}\n`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
