// Tests for test-shards.mjs: that every backend test is in exactly one CI
// shard (1cC-D1b), and that the workflow runs the shards and counts them in
// "CI result". Run with: node --test .github/scripts/

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SHARDS, assignmentProblems, commands, discover, libModules, listProblems, parseList, runsIn, tomlTargets } from "./test-shards.mjs";

const workflow = readFileSync(new URL("../workflows/ci-windows.yml", import.meta.url), "utf8").replace(/\r\n/g, "\n");

// A small crate: shard 1 filters for a and b, shard 2 is the catch-all.
const small = [
  { lib: ["a", "b"] },
  { catchAll: true, lib: ["c", "tests"], tests: ["it"], bins: true, examples: ["ex"], doc: true },
];
const smallFound = { lib: ["a", "b", "c", "tests"], tests: ["it"], bins: true, examples: ["ex"] };

test("the real crate: every module, integration test, example and the doc tests are in exactly one shard", () => {
  assert.deepEqual(assignmentProblems(discover()), []);
});

test("what discover finds is what the crate has", () => {
  const found = discover();
  assert.ok(found.lib.includes("db") && found.lib.includes("tests") && found.lib.includes("preview"));
  assert.ok(found.tests.includes("end_to_end") && found.tests.includes("csp"));
  assert.equal(found.bins, true);
  assert.deepEqual(found.examples, ["preview_seed"]);
});

test("modules are read from lib.rs, cfg'd ones and the inline tests module included", () => {
  const lib = "//! docs with mod inside\npub mod a;\n#[cfg(debug_assertions)]\npub mod b;\nmod c;\npub(crate) mod d;\n  mod indented_is_not_top_level {}\n#[cfg(test)]\nmod tests {\n}\n";
  assert.deepEqual(libModules(lib), ["a", "b", "c", "d", "tests"]);
});

test("example and test tables are read from Cargo.toml", () => {
  const toml = '[lib]\nname = "x"\n\n[[example]]\nname = "yes"\ntest = true\n\n[[example]]\nname = "no"\n\n[[test]]\nname = "t"\n\n[dependencies]\nfoo = "1"\n';
  assert.deepEqual(tomlTargets(toml, "example"), [{ name: "yes", test: true }, { name: "no", test: false }]);
  assert.deepEqual(tomlTargets(toml, "test"), [{ name: "t", test: false }]);
  assert.deepEqual(tomlTargets(toml, "bin"), []);
});

test("a module in no shard fails the check, so a new module can't be left out of CI", () => {
  const problems = assignmentProblems({ ...smallFound, lib: [...smallFound.lib, "brand_new"] }, small);
  assert.deepEqual(problems, ['The module "brand_new" is in no shard of test-shards.mjs, so its tests would never run in CI.']);
});

test("a new integration test or example in no shard fails the check", () => {
  assert.match(assignmentProblems({ ...smallFound, tests: ["it", "new_scale"] }, small)[0], /"new_scale" is in no shard/);
  assert.match(assignmentProblems({ ...smallFound, examples: ["ex", "ex2"] }, small)[0], /"ex2" is in no shard/);
});

test("a module in two shards, or one that no longer exists, fails the check", () => {
  const twice = [{ lib: ["a", "b"] }, { ...small[1], lib: ["b", "c", "tests"] }];
  assert.deepEqual(assignmentProblems(smallFound, twice), ['The module "b" is in shards 1 and 2; it must be in exactly one.']);
  const gone = [{ lib: ["a", "b", "old"] }, small[1]];
  assert.deepEqual(assignmentProblems(smallFound, gone), ['Shard 1 names the module "old", which doesn\'t exist.']);
});

test("the doc tests, the bin and the catch-all each belong to exactly one shard", () => {
  assert.match(assignmentProblems(smallFound, [small[0], { ...small[1], doc: false }])[0], /doc tests/);
  assert.match(assignmentProblems(smallFound, [{ ...small[0], doc: true }, small[1]])[0], /doc tests/);
  assert.match(assignmentProblems(smallFound, [small[0], { ...small[1], bins: false }])[0], /bin/);
  assert.match(assignmentProblems(smallFound, [small[0], { ...small[1], catchAll: false }])[0], /catch-all/);
});

test("a shard filters for its modules; the catch-all skips everyone else's", () => {
  assert.deepEqual(commands(small[0], small), [["--lib", "--", "a::", "b::"]]);
  assert.deepEqual(commands(small[1], small), [
    ["--lib", "--", "--skip", "a::", "--skip", "b::"],
    ["--test", "it", "--bins", "--example", "ex"],
    ["--doc"],
  ]);
});

test("a shard's skip keeps out a test another shard's module owns, though its filter matches", () => {
  const shards = [{ lib: ["a", "x"] }, { lib: ["b"], skip: ["a::"] }, { catchAll: true, lib: ["c", "tests"] }];
  assert.deepEqual(commands(shards[1], shards), [["--lib", "--", "b::", "--skip", "a::"]]);
  assert.deepEqual(commands(shards[2], shards), [["--lib", "--", "--skip", "a::", "--skip", "x::", "--skip", "b::"]]);
  // b's filter matches a's test a::b::t, and the skip hands it to shard 1; the catch-all skips it too.
  assert.deepEqual(listProblems(["a::b::t", "b::t", "c::t"], shards), []);
  assert.deepEqual(shards.map((s) => runsIn(s, "a::b::t", shards)), [true, false, false]);
});

test("the real shards: shard 3 runs the integration tests, the bin, the example and the doc tests", () => {
  const all = SHARDS.flatMap((s) => commands(s));
  assert.equal(all.filter((c) => c[0] === "--doc").length, 1);
  assert.equal(all.flat().filter((a) => a === "--bins").length, 1);
  assert.equal(all.flat().filter((a) => a === "--test").length, discover().tests.length);
});

test("a test runs in exactly one shard, the one that names its module", () => {
  const names = ["a::tests::x", "b::y::z", "c::q", "tests::top_level"];
  assert.deepEqual(listProblems(names, small), []);
  assert.deepEqual(names.map((n) => small.map((s) => runsIn(s, n, small))), [
    [true, false],
    [true, false],
    [false, true],
    [false, true],
  ]);
});

test("a filter that also matches a nested module's tests is caught: it would run twice or in the wrong shard", () => {
  // `a::` is a substring of `c::a::inner`, so shard 1 picks up a test of module c.
  const problems = listProblems(["c::a::inner", "a::ok"], small);
  assert.deepEqual(problems, ['c::a::inner runs in shard 1, but its module "c" is listed in shard 2.']);
});

test("a test of a module nobody lists is caught, even though the catch-all would run it", () => {
  assert.deepEqual(listProblems(["brand_new::tests::x"], small), ['brand_new::tests::x runs in shard 2, but its module "brand_new" is in no shard\'s list.']);
});

test("a test matched by no shard is caught (a stale catch-all would hide it)", () => {
  // Two plain filtering shards leave anything else matched by neither.
  const plain = [{ lib: ["a"] }, { lib: ["b"] }];
  assert.deepEqual(listProblems(["c::x"], plain), ["c::x runs in no shard."]);
  assert.deepEqual(listProblems(["a::b::x"], plain), ["a::b::x runs in shards 1 and 2."]);
});

test("the real shards: nested module names that look like another shard's filter are not in the way", () => {
  // Spot checks of the collisions the check exists for.
  assert.deepEqual(listProblems(["db::tests::x", "scan::tests::chain::y", "tests::mock_app_works", "ipc::tests::z", "matching::tests::local_corpus::w"]), []);
  assert.match(listProblems(["ipc::db::x"])[0], /ipc::db::x runs in shard 1, but its module "ipc" is listed in shard 3/);
});

test("problems are grouped by module, with a count", () => {
  const names = [...Array.from({ length: 30 }, (_, i) => `new${i % 2}::x${i}`)];
  const problems = listProblems(names, small);
  assert.equal(problems.length, 2);
  assert.equal(problems[0], `new0::x0 and 14 more runs in shard 2, but its module "new0" is in no shard's list.`);
});

test("cargo's test list is read as test names", () => {
  const out = "scan::tests::a: test\r\ndb::b: test\nsome::bench: benchmark\n\n3 tests, 1 benchmarks\n";
  assert.deepEqual(parseList(out), ["scan::tests::a", "db::b"]);
});

// The workflow.

// The windows job's text, up to the next job.
const windows = (() => {
  const start = workflow.indexOf("\n  windows:\n");
  assert.ok(start >= 0, "no windows job");
  const rest = workflow.slice(start + 1);
  const end = rest.slice(1).search(/\n {2}[A-Za-z0-9_-]+:\n/);
  return rest.slice(0, end + 1);
})();

// The switch every job's runs-on uses (runner-tools.test.mjs keeps those equal).
const SELF =
  "(vars.CI_RUNNER == 'self' && (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && github.event.pull_request.user.login != 'dependabot[bot]')";

test("the windows job runs a part per shard on GitHub's runners, and one part for everything on the owner's PC", () => {
  const matrix = windows.match(/^ {6}matrix: \$\{\{ fromJSON\((.*)\) \}\}$/m)[1];
  assert.ok(matrix.startsWith(`${SELF} && `), "the matrix uses the runs-on switch");
  const [self, github] = matrix
    .slice(SELF.length + 4)
    .split(" || ")
    .map((s) => JSON.parse(s.replace(/^'|'$/g, "")));
  assert.deepEqual(self, { include: [{ part: "all" }] });
  assert.deepEqual(github.include, [...SHARDS.map((_, i) => ({ part: "tests", shard: i + 1 })), { part: "build" }]);
  assert.match(windows, /^ {6}fail-fast: false$/m);
});

test("each shard runs the shard script with its number; the build part compiles and lints", () => {
  assert.match(windows, /if: matrix\.part == 'tests'\n(?: {8}env:\n {10}.*\n)? {8}run: node \.github\/scripts\/test-shards\.mjs run \$\{\{ matrix\.shard \}\}/);
  assert.match(windows, /name: Compile the app\n {8}if: matrix\.part != 'tests'/);
  assert.match(windows, /name: Clippy\n {8}if: \$\{\{ !cancelled\(\) && matrix\.part != 'tests' \}\}/);
  // The unsharded run is for the owner's PC alone.
  assert.match(windows, /name: Test the backend\n {8}if: matrix\.part == 'all'\n {8}working-directory: src-tauri\n {8}run: cargo test --locked\n/);
});

test("every part keeps its own rust-cache, so the shards' and the build's dependencies don't overwrite each other", () => {
  assert.match(windows, /Swatinem\/rust-cache@[^\n]+\n {8}if: runner\.environment == 'github-hosted'\n {8}with:\n {10}workspaces: src-tauri\n {10}key: \$\{\{ matrix\.part \}\}\n/);
});

test("CI result still waits for the windows job, which fails if any of its parts fails or is cancelled", () => {
  assert.match(workflow, /^ {4}needs: \[[^\]]*\bwindows\b[^\]]*\]$/m);
});
