// Splits the backend tests (cargo test in src-tauri) across parallel CI jobs
// (1cC-D1b), and proves that every test runs in exactly one of them.
//
// Why plain cargo and not cargo-nextest: nextest isn't on GitHub's Windows
// image, installing it is a new dependency, and it runs one process per test,
// which would rebuild the process-wide test fixtures (the matching corpus,
// the scan template database) for every test. `cargo test` filters give the
// same split with no behaviour change: libtest runs a test when its full name
// contains one of the filters, and not when it contains a --skip.
//
// The split, in SHARDS below:
//   - The library's unit tests (about 4,100) are split by top-level module.
//     Shards 1 and 2 name their modules, as the filters `module::`. The last
//     shard is the catch-all: it runs the library tests that none of the
//     others' filters match (a --skip for each), so a test can't fall
//     between shards, but it must still name its modules too (see below).
//   - The integration tests (src-tauri/tests), the bin's tests, the example
//     with `test = true`, and the doc tests are named by the shard that runs
//     them.
//
// Nothing is left to chance. `check` (run in the CI script tests, on every
// run) fails if a module, integration test, example or the doc tests exist
// but no shard names them, or two do, or a shard names one that is gone. And
// `run` first lists the library's tests and fails unless each runs in exactly
// one shard, and in the shard that names its top-level module: a filter such
// as `db::` also matches `ipc::db::x`, and that would run the test twice.
//
// Usage (in CI):  node .github/scripts/test-shards.mjs run <shard number>
//
//                 node .github/scripts/test-shards.mjs check

import { spawnSync } from "node:child_process";
import { appendFileSync, existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const CRATE = join(ROOT, "src-tauri");

/**
 * The shards. `lib`: top-level modules of the library whose unit tests the
 * shard runs. `tests`: integration test binaries (src-tauri/tests). `bins`:
 * the app binary's own tests. `examples`: examples with `test = true`.
 * `skip`: substrings of tests this shard must not run even though a filter
 * matches them, because they belong to a module in another shard (libtest
 * lets a --skip win over a filter). `doc`: the doc tests. `catchAll`: runs every library test the other
 * shards' filters don't match, instead of filtering for its own modules.
 *
 * Keep the shards' run times level: the slowest one is the CI time. Summed
 * test seconds per module, measured on GitHub's runner (1cC-D1b), to move
 * modules by: scan 192, matching 158, ops 127, db 110, grouping 88, quality 87,
 * relink 85, fingerprint 78, send 67, review 47, jobs 39, conflicts 33,
 * versions 31, track_table 28, crates 24, rekordbox 21, overview 21, library 20;
 * all the others under 20 each (about 110 together). A shard's test time is
 * this divided by the 4 cores. The last shard compiles the integration tests
 * and runs them and the doc tests (about 3 minutes in all), and no library
 * tests, so that no shard compiles both.
 */
export const SHARDS = [
  {
    // db's migrations::schema modules are named analysis_never_conflicts,
    // file_quality, grouping and library, so the filters "conflicts::",
    // "quality::", "grouping::" and "library::" (and the modules "quality::"
    // matches, best_file and track_table) run with db.
    lib: ["db", "grouping", "conflicts", "quality", "best_file", "track_table", "versions", "crates", "rekordbox", "overview", "library"],
  },
  {
    // scan:: also matches write_guard's, and matching:: fingerprint's.
    lib: ["scan", "write_guard", "matching", "fingerprint", "jobs"],
  },
  {
    // Everything else, by default, so a test can't fall between shards. These
    // modules (ops with relink, review with its look-alikes, send with
    // after_send) have no filter, so nothing here has to avoid a clash.
    catchAll: true,
    lib: [
      "after_send", "all_music", "attach", "audio_preview", "conflict_review", "energy", "file_roles", "fragile", "hash",
      "hidden", "ipc", "missing", "net", "ops", "paths", "preview", "read", "rekordbox_write", "relink", "review",
      "scan_state", "search", "send", "send_values", "settings", "sniff", "start", "suggest", "tags", "ui_setting",
      "volume", "waveform",
      // The tests in lib.rs itself are named tests::..., which no filter can pick
      // out (every `scan::tests::x` contains "tests::"), so only the catch-all can run them.
      "tests",
    ],
  },
  {
    // No library tests: this shard compiles the integration tests, and a shard
    // that also compiled the library's test binary would be the slowest.
    lib: [],
    tests: [
      "csp", "end_to_end", "file_roles_scale", "fingerprint_memory", "freeze_prototype", "grouping_scale",
      "matching_memory", "net_gate", "perf_100k", "rekordbox_xml_scale", "search_scale", "track_table_scale",
    ],
    bins: true,
    examples: ["preview_seed"],
    doc: true,
  },
];

const filtersOf = (shard) => shard.lib.map((m) => `${m}::`);

/** The argument lists of the cargo commands that run `shard` (all after `cargo test --locked --no-fail-fast`). */
export function commands(shard, shards = SHARDS) {
  const out = [];
  if (shard.lib.length > 0) {
    const others = shards.filter((s) => s !== shard);
    const skips = (shard.catchAll ? others.flatMap(filtersOf) : shard.skip ?? []).flatMap((f) => ["--skip", f]);
    const args = [...(shard.catchAll ? [] : filtersOf(shard)), ...skips];
    out.push(["--lib", "--", ...args]);
  }
  const targets = [
    ...(shard.tests ?? []).flatMap((t) => ["--test", t]),
    ...(shard.bins ? ["--bins"] : []),
    ...(shard.examples ?? []).flatMap((e) => ["--example", e]),
  ];
  if (targets.length > 0) out.push(targets);
  // cargo refuses --doc together with other target selections.
  if (shard.doc) out.push(["--doc"]);
  return out;
}

/** Whether libtest runs the test `name` in `shard` (filters match as substrings of the full name). */
export function runsIn(shard, name, shards = SHARDS) {
  if (shard.catchAll) return !shards.some((s) => s !== shard && filtersOf(s).some((f) => name.includes(f)));
  return filtersOf(shard).some((f) => name.includes(f)) && !(shard.skip ?? []).some((f) => name.includes(f));
}

// What exists in src-tauri, from its files.

/** Top-level modules of the library: the `mod` items at the start of lib.rs's lines. */
export function libModules(libRs) {
  return [...libRs.matchAll(/^(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)/gm)].map((m) => m[1]);
}

/** The `[[kind]]` tables of a Cargo.toml: [{ name, test }]. */
export function tomlTargets(cargoToml, kind) {
  return cargoToml
    .split(/^\[/m)
    .filter((block) => block.startsWith(`[${kind}]]`))
    .map((block) => ({
      name: block.match(/^name\s*=\s*"([^"]+)"/m)?.[1],
      test: block.match(/^test\s*=\s*(true|false)/m)?.[1] === "true",
    }));
}

/** Everything in the crate that `cargo test` runs: { lib, tests, bins, examples } (lib has doc tests). */
export function discover(crate = CRATE) {
  const read = (p) => readFileSync(join(crate, p), "utf8").replace(/\r\n/g, "\n");
  const cargo = read("Cargo.toml");
  const tests = new Set(tomlTargets(cargo, "test").map((t) => t.name));
  const dir = join(crate, "tests");
  if (existsSync(dir)) {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) {
        if (existsSync(join(full, "main.rs"))) tests.add(entry);
      } else if (entry.endsWith(".rs")) {
        tests.add(entry.slice(0, -3));
      }
    }
  }
  const hasBin =
    existsSync(join(crate, "src", "main.rs")) ||
    existsSync(join(crate, "src", "bin")) ||
    tomlTargets(cargo, "bin").length > 0;
  return {
    lib: libModules(read("src/lib.rs")),
    tests: [...tests],
    bins: hasBin,
    examples: tomlTargets(cargo, "example").filter((e) => e.test).map((e) => e.name),
  };
}

/** Why `shards` doesn't cover exactly `found` (from discover): one message per problem; empty if it does. */
export function assignmentProblems(found, shards = SHARDS) {
  const problems = [];
  const where = (key, name) => shards.flatMap((s, i) => ((s[key] ?? []).includes(name) ? [i + 1] : []));
  const kinds = [
    ["lib", "module", "lib"],
    ["tests", "integration test (src-tauri/tests)", "tests"],
    ["examples", "example with test = true", "examples"],
  ];
  for (const [key, label] of kinds) {
    for (const name of found[key]) {
      const at = where(key, name);
      if (at.length === 0) problems.push(`The ${label} "${name}" is in no shard of test-shards.mjs, so its tests would never run in CI.`);
      if (at.length > 1) problems.push(`The ${label} "${name}" is in shards ${at.join(" and ")}; it must be in exactly one.`);
    }
    const known = new Set(found[key]);
    for (const [i, s] of shards.entries()) {
      for (const name of s[key] ?? []) {
        if (!known.has(name)) problems.push(`Shard ${i + 1} names the ${label} "${name}", which doesn't exist.`);
      }
    }
  }
  const count = (key) => shards.filter((s) => s[key]).length;
  if (found.bins && count("bins") !== 1) problems.push(`The bin's tests must be in exactly one shard (found ${count("bins")}).`);
  if (!found.bins && count("bins") > 0) problems.push("A shard runs --bins but the crate has no bin.");
  // The doc tests belong to the library; one shard runs them.
  if (count("doc") !== 1) problems.push(`The doc tests must be in exactly one shard (found ${count("doc")}).`);
  const catchAll = shards.filter((s) => s.catchAll).length;
  if (catchAll !== 1) problems.push(`Exactly one shard must be the catch-all (found ${catchAll}).`);
  return problems;
}

/** The test names in `cargo test -- --list` output (`path::to::test: test`). */
export function parseList(output) {
  return output
    .split(/\r?\n/)
    .map((l) => l.match(/^(.+): test$/)?.[1])
    .filter(Boolean);
}

/**
 * Why the library's tests (`names`) don't each run in exactly one shard, and
 * in the shard that names their module; empty if they do. One message per
 * module and wrong place, with a count and the first test as an example.
 */
export function listProblems(names, shards = SHARDS) {
  const groups = new Map();
  for (const name of names) {
    const module = name.split("::")[0];
    const at = shards.flatMap((s, i) => (runsIn(s, name, shards) ? [i + 1] : []));
    const named = shards.flatMap((s, i) => (s.lib.includes(module) ? [i + 1] : []));
    let why = null;
    if (at.length !== 1) {
      why = `runs in ${at.length === 0 ? "no shard" : `shards ${at.join(" and ")}`}`;
    } else if (named.length !== 1 || named[0] !== at[0]) {
      why = `runs in shard ${at[0]}, but its module "${module}" is ${named.length === 0 ? "in no shard's list" : `listed in shard ${named.join(" and ")}`}`;
    }
    if (!why) continue;
    const key = `${module}|${why}`;
    const group = groups.get(key) ?? { why, count: 0, first: name };
    group.count++;
    groups.set(key, group);
  }
  return [...groups.values()].map((g) => `${g.first}${g.count > 1 ? ` and ${g.count - 1} more` : ""} ${g.why}.`);
}

// Reports.

function summary(text) {
  console.log(text);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${text}\n`);
}

function fail(problems) {
  for (const p of problems) console.error(`::error::${p}`);
  process.exit(1);
}

function cargo(args, options = {}) {
  return spawnSync("cargo", ["test", "--locked", "--no-fail-fast", ...args], { cwd: CRATE, ...options });
}

function runShard(number) {
  const shard = SHARDS[number - 1];
  if (!shard) fail([`There is no shard ${number}; there are ${SHARDS.length}.`]);
  fail0(assignmentProblems(discover()));

  if (shard.lib.length > 0) {
    // Compiles the library's test binary, which the run below reuses.
    const listed = cargo(["--lib", "--", "--list"], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
    if (listed.status !== 0) fail(["Listing the library's tests failed."]);
    const names = parseList(listed.stdout);
    if (names.length === 0) fail(["The library's test list is empty."]);
    fail0(listProblems(names));
    const mine = names.filter((n) => runsIn(shard, n));
    summary(`Shard ${number}/${SHARDS.length}: ${mine.length} of the library's ${names.length} tests (every one runs in exactly one shard).`);
  }

  let failed = false;
  for (const args of commands(shard)) {
    console.log(`::group::cargo test ${args.join(" ")}`);
    console.log("::endgroup::");
    const result = cargo(args, { stdio: "inherit" });
    if (result.status !== 0) failed = true;
  }
  if (failed) fail([`Shard ${number}: a test failed.`]);
}

function fail0(problems) {
  if (problems.length > 0) fail(problems);
}

function main() {
  const [command, arg] = process.argv.slice(2);
  if (command === "run") return runShard(Number(arg));
  if (command === "check") {
    fail0(assignmentProblems(discover()));
    return console.log(`Every module, integration test, example and doc test is in exactly one of the ${SHARDS.length} shards.`);
  }
  console.error("Usage: test-shards.mjs run <shard> | check");
  process.exit(2);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
