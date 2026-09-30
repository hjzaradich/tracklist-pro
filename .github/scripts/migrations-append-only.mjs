// Fails a pull request that changes migration history (CLAUDE.md: migrations
// are append-only). The Rust append-only test compares each migration with
// its line in src-tauri/migrations/checksums.txt, so a PR that edits both
// would pass it; this check compares the PR with its base branch instead.
// Relative to the merge base, the PR may only:
//   - add new src-tauri/migrations/*.sql files (never modify, rename or
//     delete one), and
//   - add lines to checksums.txt (never change or remove one; comment and
//     blank lines are free to change).
// A base without the folder or the file passes: everything is an addition.
//
// Usage: node .github/scripts/migrations-append-only.mjs <base-ref>
// e.g. origin/main. Needs enough history for `git merge-base`.
// Exits 1 and lists every problem if the PR rewrites history.

import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const MIGRATIONS_DIR = "src-tauri/migrations";
export const CHECKSUMS = `${MIGRATIONS_DIR}/checksums.txt`;

/**
 * Takes `git diff --name-status --no-renames -z` output (NUL-separated, so
 * unusual file names aren't quoted) and returns a problem for every
 * migration .sql file that isn't a plain addition. With renames off, a
 * rename is a delete plus an add, so the delete is caught.
 */
export function sqlProblems(nameStatus) {
  const problems = [];
  const fields = nameStatus.split("\0");
  for (let i = 0; i + 1 < fields.length; i += 2) {
    const [status, path] = [fields[i], fields[i + 1]];
    if (!path.startsWith(`${MIGRATIONS_DIR}/`) || !path.endsWith(".sql")) continue;
    if (status !== "A") {
      const what = { M: "modified", D: "deleted or renamed", T: "changed type" }[status] ?? `status ${status}`;
      problems.push(`${path}: ${what}; migrations on the base branch are append-only`);
    }
  }
  return problems;
}

/**
 * Takes the `git diff` of checksums.txt and returns a problem for every
 * removed line (a changed line is a removal plus an addition). Comment and
 * blank lines don't record a migration, so they're ignored.
 */
export function checksumProblems(diff) {
  const problems = [];
  let inHunk = false;
  for (const line of diff.split("\n")) {
    if (line.startsWith("@@")) {
      inHunk = true;
      continue;
    }
    if (line.startsWith("diff --git")) {
      inHunk = false;
      continue;
    }
    if (!inHunk || !line.startsWith("-")) continue;
    const removed = line.slice(1).trim();
    if (removed === "" || removed.startsWith("#")) continue;
    problems.push(`${CHECKSUMS}: line changed or removed: ${removed}`);
  }
  return problems;
}

function git(args, cwd) {
  return execFileSync("git", args, { cwd, encoding: "utf8" });
}

/** Returns every problem between the merge base of `baseRef` and HEAD. */
export function check(baseRef, cwd = process.cwd()) {
  const base = git(["merge-base", baseRef, "HEAD"], cwd).trim();
  const nameStatus = git(["diff", "--name-status", "--no-renames", "-z", base, "HEAD", "--", MIGRATIONS_DIR], cwd);
  const diff = git(["diff", "--no-renames", "--no-ext-diff", "-U0", base, "HEAD", "--", CHECKSUMS], cwd);
  return [...sqlProblems(nameStatus), ...checksumProblems(diff)];
}

function main([baseRef]) {
  if (!baseRef) {
    console.error("usage: migrations-append-only.mjs <base-ref>");
    process.exit(2);
  }
  const problems = check(baseRef);
  if (problems.length > 0) {
    console.error(`Migration history changed relative to ${baseRef}:`);
    for (const p of problems) console.error(`  ${p}`);
    console.error("Add a new migration (and a new checksums.txt line) instead of editing an old one.");
    process.exit(1);
  }
  console.log(`migrations ok: only additions relative to ${baseRef}`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
