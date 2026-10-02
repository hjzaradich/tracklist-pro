// Tests for runner-tools.mjs, and for how ci-windows.yml switches between
// GitHub's runners and the owner's self-hosted one (1aA-16). Run with:
// node --test .github/scripts/

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  TOOLS,
  CAPABILITIES,
  CARGO_DENY_ACTION,
  TARGET_WARN_BYTES,
  needProblems,
  rustRelease,
  rustProblems,
  stableUpdate,
  envLines,
  targetDir,
  canSymlink,
  cargoDenyVersion,
  cargoDenyProblems,
  older,
  dirSize,
  targetWarning,
  parseArgs,
  isCold,
  coldWarning,
} from "./runner-tools.mjs";

const SCRIPT = fileURLToPath(new URL("./runner-tools.mjs", import.meta.url));

// rustc -vV, trimmed to the lines that matter.
const rustc = (release) => `rustc ${release} (88d9e12ae 2026-08-18)\nbinary: rustc\nhost: x86_64-pc-windows-msvc\nrelease: ${release}\nLLVM version: 22.1.8\n`;

test("a stable Rust passes; a beta or nightly default fails, since CI means stable", () => {
  assert.deepEqual(rustRelease(rustc("1.98.0")), { release: "1.98.0", channel: "stable" });
  assert.deepEqual(rustProblems(rustc("1.98.0")), []);
  assert.deepEqual(rustRelease(rustc("1.99.0-beta.3")), { release: "1.99.0", channel: "beta" });
  assert.deepEqual(rustRelease(rustc("1.100.0-nightly")), { release: "1.100.0", channel: "nightly" });
  assert.match(rustProblems(rustc("1.99.0-beta.3"))[0], /the default Rust is beta 1\.99\.0/);
  assert.match(rustProblems(rustc("1.100.0-nightly"))[0], /nightly/);
});

test("rustc output without a release line is a problem, not a pass", () => {
  assert.equal(rustRelease("rustc 1.98.0\n"), null);
  assert.match(rustProblems("")[0], /can't read rustc's release/);
});

test("rustup check: a stable update is found in the current and the older wording", () => {
  const now = "stable-x86_64-pc-windows-msvc - update available: 1.98.0 (88d9e12ae 2026-08-18) -> 1.98.1 (48a229cea 2026-09-01)\nrustup - update available : 1.29.0 -> 1.29.1\n";
  assert.deepEqual(stableUpdate(now), { from: "1.98.0", to: "1.98.1" });
  const older = "stable-x86_64-pc-windows-msvc - Update available : 1.97.0 (aaaaaaaaa 2026-07-01) -> 1.98.0 (88d9e12ae 2026-08-18)\n";
  assert.deepEqual(stableUpdate(older), { from: "1.97.0", to: "1.98.0" });
});

test("rustup check: up to date, or only a rustup or nightly update, is no stable update", () => {
  assert.equal(stableUpdate("stable-x86_64-pc-windows-msvc - Up to date : 1.98.0 (88d9e12ae 2026-08-18)\n"), null);
  assert.equal(stableUpdate("rustup - update available : 1.29.0 -> 1.29.1\n"), null);
  assert.equal(stableUpdate("nightly-x86_64-pc-windows-msvc - update available: 1.100.0 -> 1.100.0\n"), null);
  assert.equal(stableUpdate(""), null);
});

test("the build output goes outside the checkout", () => {
  const repoDir = join("C:", "actions-runner", "_work", "tracklist-pro");
  const checkout = join(repoDir, "tracklist-pro");
  assert.equal(targetDir(checkout), join(repoDir, "cargo-target"));
  assert.deepEqual(envLines(checkout), [`CARGO_TARGET_DIR=${join(repoDir, "cargo-target")}`]);
  // A sibling of the checkout, so checkout's clean never deletes it.
  assert.ok(!targetDir(checkout).startsWith(checkout));
});

test("cargo-deny passes at or above the action's version and fails below it", () => {
  assert.equal(cargoDenyVersion("cargo-deny 0.20.2"), "0.20.2");
  assert.equal(cargoDenyVersion("error: no such command: `deny`"), null);
  assert.deepEqual(cargoDenyProblems(`cargo-deny ${CARGO_DENY_ACTION.version}`), []);
  assert.deepEqual(cargoDenyProblems("cargo-deny 0.20.10"), []);
  assert.deepEqual(cargoDenyProblems("cargo-deny 1.0.0"), []);
  assert.match(cargoDenyProblems("cargo-deny 0.19.9")[0], /older than the 0\.20\.2 GitHub's runners use/);
  assert.match(cargoDenyProblems("cargo-deny 0.20.1")[0], /cargo install --locked cargo-deny/);
  assert.match(cargoDenyProblems("")[0], /can't read cargo-deny's version/);
});

test("versions compare by number, not as text", () => {
  assert.equal(older("0.20.2", "0.20.10"), true);
  assert.equal(older("0.20.10", "0.20.2"), false);
  assert.equal(older("0.20.2", "0.20.2"), false);
  assert.equal(older("0.9.0", "0.20.0"), true);
  assert.equal(older("1.0.0", "0.99.99"), false);
});

test("the version floor is the cargo-deny that the pinned cargo-deny-action bundles", () => {
  // Bumping the action changes its pin; this fails until CARGO_DENY_ACTION
  // is updated with the new commit and its cargo-deny (its Dockerfile).
  const pins = [...readWorkflow("ci-windows.yml").matchAll(/EmbarkStudios\/cargo-deny-action@([0-9a-f]{40})/g)].map((m) => m[1]);
  assert.equal(pins.length, 2);
  for (const pin of pins) assert.equal(pin, CARGO_DENY_ACTION.sha);
});

test("a build folder above 20 GB gets a warning with the cargo clean command", () => {
  assert.equal(targetWarning("C:\\t", TARGET_WARN_BYTES), null);
  assert.equal(targetWarning("C:\\t", 12e9), null);
  const warning = targetWarning("C:\\t", 25.3e9);
  assert.match(warning, /holds 25\.3 GB/);
  assert.match(warning, /cargo clean --target-dir "C:\\t"/);
});

test("--builds= names a package the job compiles; other options are refused", () => {
  assert.deepEqual(parseArgs(["rust", "clippy", "--builds=tracklist-pro"]), { needs: ["rust", "clippy"], builds: ["tracklist-pro"], problems: [] });
  assert.deepEqual(parseArgs(["node"]), { needs: ["node"], builds: [], problems: [] });
  assert.match(parseArgs(["rust", "--builds="]).problems[0], /unknown option "--builds="/);
  assert.match(parseArgs(["rust", "--build=x"]).problems[0], /unknown option "--build=x"/);
});

test("a build is cold until the build folder holds a fingerprint of that package", () => {
  const dir = mkdtempSync(join(tmpdir(), "runner-tools-cold-"));
  try {
    assert.equal(isCold(join(dir, "missing"), "tracklist-pro"), true);
    mkdirSync(join(dir, "debug", ".fingerprint", "serde-0123456789abcdef"), { recursive: true });
    assert.equal(isCold(dir, "tracklist-pro"), true);
    mkdirSync(join(dir, "debug", ".fingerprint", "tracklist-pro-fc18310070ef88b7"));
    assert.equal(isCold(dir, "tracklist-pro"), false);
    // Another package's build doesn't make this one warm.
    assert.equal(isCold(dir, "fixture-gen"), true);
    assert.match(coldWarning("fixture-gen"), /No earlier build of fixture-gen/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("dirSize adds up every file below a folder, and a missing folder is empty", () => {
  const dir = mkdtempSync(join(tmpdir(), "runner-tools-size-"));
  try {
    mkdirSync(join(dir, "debug", "deps"), { recursive: true });
    writeFileSync(join(dir, "a"), Buffer.alloc(100));
    writeFileSync(join(dir, "debug", "deps", "b"), Buffer.alloc(250));
    assert.equal(dirSize(dir), 350);
    assert.equal(dirSize(join(dir, "missing")), 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("needs must be named, and only known ones", () => {
  assert.deepEqual(needProblems(["rust", "clippy", "symlinks"]), []);
  assert.match(needProblems([])[0], /no needs given/);
  assert.match(needProblems(["rust", "cargo-denny"])[0], /unknown need "cargo-denny"/);
});

test("canSymlink answers yes or no, never throws", () => {
  assert.equal(typeof canSymlink(), "boolean");
});

test("the script fails on an unknown need, but still moves the build output out of the checkout first", () => {
  const dir = mkdtempSync(join(tmpdir(), "runner-tools-test-"));
  try {
    const envFile = join(dir, "github_env");
    const r = spawnSync(process.execPath, [SCRIPT, "node", "bogus"], {
      encoding: "utf8",
      env: { ...process.env, GITHUB_ENV: envFile, GITHUB_WORKSPACE: join(dir, "repo", "checkout") },
    });
    assert.equal(r.status, 2, r.stderr);
    assert.match(r.stderr, /unknown need "bogus"/);
    assert.doesNotMatch(r.stdout, /node --version/);
    assert.equal(readFileSync(envFile, "utf8"), `CARGO_TARGET_DIR=${join(dir, "repo", "cargo-target")}\n`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the script prints versions, passes, and hands the job its environment", () => {
  const dir = mkdtempSync(join(tmpdir(), "runner-tools-test-"));
  try {
    const envFile = join(dir, "github_env");
    const ws = join(dir, "repo", "checkout");
    const out = execFileSync(process.execPath, [SCRIPT, "node", "git"], {
      encoding: "utf8",
      env: { ...process.env, GITHUB_ENV: envFile, GITHUB_WORKSPACE: ws },
    });
    assert.match(out, /node --version: v\d+/);
    assert.match(out, /git --version: git version/);
    assert.equal(readFileSync(envFile, "utf8"), `CARGO_TARGET_DIR=${join(dir, "repo", "cargo-target")}\n`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// The workflow, read from ci-windows.yml. A small reader, enough for this
// file's layout: job ids at two spaces under `jobs:`, job keys at four,
// steps as `      - ` items at six.

function readWorkflow(name) {
  return readFileSync(new URL(`../workflows/${name}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
}

function workflowJobs(yml) {
  const lines = yml.split("\n");
  const jobs = {};
  let job = null;
  let step = null;
  for (const line of lines.slice(lines.indexOf("jobs:") + 1)) {
    if (/^\S/.test(line)) break;
    const id = line.match(/^ {2}([A-Za-z0-9_-]+):\s*$/);
    if (id) {
      job = jobs[id[1]] = { runsOn: "", timeout: "", steps: [] };
      step = null;
      continue;
    }
    const runsOn = line.match(/^ {4}runs-on:\s*(.*)$/);
    if (job && runsOn) job.runsOn = runsOn[1];
    const timeout = line.match(/^ {4}timeout-minutes:\s*(.*)$/);
    if (job && timeout) job.timeout = timeout[1];
    if (/^ {4}\S/.test(line)) step = null;
    if (/^ {6}- /.test(line)) {
      step = { text: "" };
      job.steps.push(step);
    }
    if (step && /^ {6}/.test(line) && !/^\s*#/.test(line)) step.text += `${line}\n`;
  }
  return jobs;
}

const field = (step, key) => step.text.match(new RegExp(`^ {8}${key}:\\s*(.*)$`, "m"))?.[1] ?? null;
const uses = (step) => (step.text.match(/^ {6}- uses:\s*(\S+)|^ {8}uses:\s*(\S+)/m) ?? []).slice(1).find(Boolean) ?? null;
const run = (step) => {
  const m = step.text.match(/^ {8}run:\s*(.*)$/m);
  if (!m) return null;
  if (m[1] !== "|") return m[1];
  return step.text.slice(m.index + m[0].length).split("\n").filter((l) => /^ {10}/.test(l)).map((l) => l.trim()).join("\n");
};

const yml = readWorkflow("ci-windows.yml");
const jobs = workflowJobs(yml);

// The one switch every job uses: self-hosted only when the variable says so,
// and the run isn't a pull request from a fork or from Dependabot.
const SELF =
  "(vars.CI_RUNNER == 'self' && (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && github.event.pull_request.user.login != 'dependabot[bot]')";
const runsOn = (fallback) => `\${{ ${SELF} && fromJSON('["self-hosted", "tlp-windows"]') || '${fallback}' }}`;
const GITHUB_HOSTED = {
  changes: "ubuntu-latest",
  windows: "windows-latest",
  "fixture-gen": "windows-latest",
  "rb-kit": "windows-latest",
  checks: "ubuntu-latest",
  licenses: "ubuntu-latest",
  result: "ubuntu-latest",
};

test("every job switches runners with the same expression, keeping its GitHub runner as the default", () => {
  // Guards the reader itself: it must find every job.
  assert.deepEqual(Object.keys(jobs).sort(), Object.keys(GITHUB_HOSTED).sort());
  for (const [id, job] of Object.entries(jobs)) {
    assert.equal(job.runsOn, runsOn(GITHUB_HOSTED[id]), `${id}: runs-on`);
  }
});

test("each job gets more time on the busy self-hosted runner, and the same time as before on GitHub's", () => {
  // Minutes: [GitHub's runners (unchanged), self-hosted].
  const minutes = {
    changes: [10, 20],
    windows: [45, 120],
    "fixture-gen": [30, 60],
    "rb-kit": [10, 20],
    checks: [15, 30],
    licenses: [15, 30],
    result: [5, 15],
  };
  assert.deepEqual(Object.keys(minutes).sort(), Object.keys(jobs).sort());
  for (const [id, [github, self]] of Object.entries(minutes)) {
    assert.equal(jobs[id].timeout, `\${{ ${SELF} && ${self} || ${github} }}`, id);
    assert.ok(self > github, id);
  }
});

test("the switch needs CI_RUNNER set to self, and never sends a fork's or Dependabot's pull request to the owner's PC", () => {
  // Evaluates the switch the way GitHub would, for the cases that matter.
  // GitHub's == ignores case when comparing strings, and a push, scheduled
  // or manual run has no pull_request (its user.login is null, so != holds).
  const eq = (a, b) => typeof a === "string" && typeof b === "string" && a.toLowerCase() === b.toLowerCase();
  const pick = ({ runner, event, head = "owner/tracklist-pro", repo = "owner/tracklist-pro", author = "owner" }) => {
    const pr = event === "pull_request";
    const login = pr ? author : null;
    return eq(runner, "self") && (!pr || eq(head, repo)) && !eq(login, "dependabot[bot]") ? "self-hosted" : "github";
  };
  assert.equal(pick({ runner: undefined, event: "pull_request" }), "github");
  assert.equal(pick({ runner: "github", event: "push" }), "github");
  assert.equal(pick({ runner: "Self", event: "push" }), "self-hosted");
  assert.equal(pick({ runner: "self", event: "push" }), "self-hosted");
  assert.equal(pick({ runner: "self", event: "schedule" }), "self-hosted");
  assert.equal(pick({ runner: "self", event: "workflow_dispatch" }), "self-hosted");
  assert.equal(pick({ runner: "self", event: "pull_request" }), "self-hosted");
  assert.equal(pick({ runner: "self", event: "pull_request", head: "someone/tracklist-pro" }), "github");
  assert.equal(pick({ runner: "self", event: "pull_request", author: "dependabot[bot]" }), "github");
  // …and the workflow's expression is that same logic.
  assert.ok(SELF.includes("vars.CI_RUNNER == 'self'"));
  assert.ok(SELF.includes("github.event.pull_request.head.repo.full_name == github.repository"));
  assert.ok(SELF.includes("github.event.pull_request.user.login != 'dependabot[bot]'"));
  // No job names self-hosted any other way.
  assert.equal((yml.match(/self-hosted", "tlp-windows/g) ?? []).length, Object.keys(jobs).length);
});

test("actions that install toolchains, prune caches or need Docker run on GitHub's runners only", () => {
  const githubOnly = ["dtolnay/rust-toolchain@", "Swatinem/rust-cache@", "EmbarkStudios/cargo-deny-action@"];
  let seen = 0;
  for (const [id, job] of Object.entries(jobs)) {
    for (const step of job.steps) {
      // The toolchain is installed by rustup from rust-toolchain.toml, a run
      // step; the others are actions.
      const action = uses(step) ?? (run(step)?.startsWith("rustup toolchain install") ? run(step) : undefined);
      if (!action || !(githubOnly.some((a) => action.startsWith(a)) || action.startsWith("rustup toolchain install"))) continue;
      seen++;
      assert.match(field(step, "if") ?? "", /runner\.environment == 'github-hosted'/, `${id}: ${action}`);
    }
  }
  // 3 toolchain installs, 2 rust-caches, 2 cargo-deny actions; guards the reader too.
  assert.equal(seen, 7);
});

test("each cargo-deny action step has a self-hosted twin that runs the same check", () => {
  let pairs = 0;
  for (const job of Object.values(jobs)) {
    for (const step of job.steps) {
      if (!uses(step)?.startsWith("EmbarkStudios/cargo-deny-action@")) continue;
      const w = (k) => step.text.match(new RegExp(`^ {10}${k}:\\s*(.*)$`, "m"))[1];
      // The action passes --log-level (default warn) before the rest.
      assert.equal(step.text.match(/^ {10}log-level:/m), null, "set log-level in the twin too");
      const expected = `cargo deny --log-level warn --manifest-path ${w("manifest-path")} ${w("arguments")} ${w("command")} ${w("command-arguments")}`;
      const twin = job.steps.find((s) => run(s) === expected);
      assert.ok(twin, `no self-hosted step runs: ${expected}`);
      assert.match(field(twin, "if"), /runner\.environment == 'self-hosted'/);
      // Both twins keep the same "run even if an earlier check failed" rule.
      assert.equal(field(twin, "if").includes("!cancelled()"), (field(step, "if") ?? "").includes("!cancelled()"));
      pairs++;
    }
  }
  assert.equal(pairs, 2);
});

test("every job first checks the self-hosted runner has each tool its steps use", () => {
  // A command at the start of a line or after a space or $( (not e.g. the
  // "npm" in npm-licenses.mjs).
  const cmd = (c) => new RegExp(`(?:^|[\\s(])${c}\\s`, "m");
  const uses_ = {
    rust: cmd("cargo"),
    clippy: cmd("cargo clippy"),
    rustfmt: cmd("cargo fmt"),
    "cargo-deny": cmd("cargo deny"),
    npm: cmd("npm"),
    python: cmd("python"),
    gh: cmd("gh"),
    git: cmd("git"),
  };
  for (const [id, job] of Object.entries(jobs)) {
    const check = job.steps.find((s) => run(s)?.startsWith("node .github/scripts/runner-tools.mjs "));
    assert.ok(check, `${id} has no runner tools check`);
    assert.equal(field(check, "if"), "runner.environment == 'self-hosted'", id);
    const { needs, builds, problems } = parseArgs(run(check).split(" ").slice(2));
    assert.deepEqual([...problems, ...needProblems(needs)], [], id);
    // A job that compiles a package names it, so a cold first build warns.
    const cargoPackage = (dir) => readFileSync(new URL(`../../${dir}/Cargo.toml`, import.meta.url), "utf8").match(/^name = "(.+)"$/m)[1];
    const compiles = { windows: [cargoPackage("src-tauri")], "fixture-gen": [cargoPackage("tools/fixture-gen")] };
    assert.deepEqual(builds, compiles[id] ?? [], `${id}: --builds=`);
    // It comes before any step that builds or tests (only checkout and
    // setup-node may run first).
    for (const before of job.steps.slice(0, job.steps.indexOf(check))) {
      assert.match(uses(before) ?? "", /^actions\/(checkout|setup-node)@/, `${id}: a step runs before the tools check`);
    }
    const commands = job.steps.map(run).filter(Boolean).filter((r) => !r.startsWith("node .github/scripts/runner-tools.mjs")).join("\n");
    for (const [need, pattern] of Object.entries(uses_)) {
      if (pattern.test(commands)) assert.ok(needs.includes(need), `${id} runs ${pattern} but doesn't check for ${need}`);
    }
  }
  // The job that runs the write guard's tests also needs file symlinks, or
  // its symlink cases would skip on the owner's PC.
  assert.ok(run(jobs.windows.steps.find((s) => run(s)?.includes("runner-tools.mjs"))).split(" ").includes("symlinks"));
  assert.ok(CAPABILITIES.includes("symlinks") && "rust" in TOOLS);
});

test("npm's cache is GitHub-hosted only; the self-hosted runner keeps its own", () => {
  let seen = 0;
  for (const job of Object.values(jobs)) {
    for (const step of job.steps) {
      if (!uses(step)?.startsWith("actions/setup-node@")) continue;
      seen++;
      const cache = step.text.match(/^ {10}cache:\s*(.*)$/m)?.[1];
      const pmCache = step.text.match(/^ {10}package-manager-cache:\s*(.*)$/m)?.[1];
      if (cache !== undefined) assert.equal(cache, "${{ runner.environment == 'github-hosted' && 'npm' || '' }}");
      assert.ok(pmCache === "false" || pmCache === "${{ runner.environment == 'github-hosted' }}", pmCache);
    }
  }
  assert.ok(seen >= 4);
});

test("rustup may never install a toolchain, in any job, the tools check included", () => {
  assert.match(yml, /^env:\n(?: {2}.*\n)*? {2}RUSTUP_AUTO_INSTALL: "0"\n/m);
  // Set once for the whole workflow, not overridden by a job or step.
  assert.equal((yml.match(/RUSTUP_AUTO_INSTALL/g) ?? []).length, 1);
});

test("every step runs in bash on every runner, never PowerShell", () => {
  assert.match(yml, /^defaults:\n {2}run:\n {4}shell: bash\n/m);
  assert.doesNotMatch(yml, /shell:\s*(pwsh|powershell|cmd)/);
});

test("a newer push to a pull request cancels its older run, so the one runner doesn't queue stale work", () => {
  assert.match(yml, /^concurrency:\n {2}group: \$\{\{ github\.workflow \}\}-\$\{\{ github\.event_name == 'pull_request' && github\.event\.pull_request\.number \|\| github\.sha \}\}\n {2}cancel-in-progress: \$\{\{ github\.event_name == 'pull_request' \}\}\n/m);
});

test("CI (macOS, Linux) never runs on pull requests or pushes, and never on the owner's PC", () => {
  // So its failures (e.g. while GitHub minutes are out) never reach a PR's checks.
  const other = readWorkflow("ci-macos-linux.yml");
  const on = other.match(/^on:\n((?: {2}.*\n|\n)*)/m)[1];
  assert.doesNotMatch(on, /^ {2}(pull_request|pull_request_target|push|workflow_run):/m);
  assert.match(on, /^ {2}schedule:/m);
  assert.doesNotMatch(other, /self-hosted|tlp-windows|CI_RUNNER/);
});
