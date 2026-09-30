// Checks every npm package in package-lock.json against the license allow
// list in deny.toml, so Rust and npm share one GPL-3.0-compatible policy.
// No dependencies: the lockfile already records each
// package's `license` field.
//
// Usage: node .github/scripts/npm-licenses.mjs [package-lock.json] [deny.toml]
// Exits 1 and lists the offending packages if any license isn't allowed.

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

/** Reads the `allow = [...]` list from the `[licenses]` table of deny.toml. */
export function parseAllowList(tomlText) {
  const lines = tomlText.split(/\r?\n/).map((l) => l.replace(/#.*$/, ""));
  const start = lines.findIndex((l) => l.trim() === "[licenses]");
  if (start < 0) throw new Error("deny.toml has no [licenses] table");
  let table = [];
  for (const line of lines.slice(start + 1)) {
    if (/^\s*\[/.test(line)) break;
    table.push(line);
  }
  const match = /(?:^|\n)\s*allow\s*=\s*\[([^\]]*)\]/.exec(table.join("\n"));
  if (!match) throw new Error("deny.toml [licenses] has no allow list");
  const ids = [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (ids.length === 0) throw new Error("deny.toml allow list is empty");
  return new Set(ids.map(normalize));
}

function normalize(id) {
  return id.trim().replace(/\s+/g, " ").toLowerCase();
}

/**
 * True if the SPDX expression can be satisfied with allowed licenses:
 * `A OR B` needs one side, `A AND B` needs both, and `A WITH exception`
 * must be allowed as written. Anything that isn't a valid SPDX expression
 * (missing, "UNLICENSED", "SEE LICENSE IN ...") is not allowed.
 */
export function isAllowed(expression, allow) {
  if (typeof expression !== "string" || expression.trim() === "") return false;
  const tokens = expression.match(/\(|\)|[^\s()]+/g) ?? [];
  let pos = 0;
  const peek = () => tokens[pos]?.toUpperCase();

  // or := and ("OR" and)*   and := atom ("AND" atom)*
  // atom := "(" or ")" | id ["WITH" id]
  function parseOr() {
    let ok = parseAnd();
    while (peek() === "OR") {
      pos++;
      ok = parseAnd() || ok;
    }
    return ok;
  }
  function parseAnd() {
    let ok = parseAtom();
    while (peek() === "AND") {
      pos++;
      ok = parseAtom() && ok;
    }
    return ok;
  }
  function parseAtom() {
    const token = tokens[pos++];
    if (token === undefined) throw new Error("unexpected end");
    if (token === "(") {
      const ok = parseOr();
      if (tokens[pos++] !== ")") throw new Error("missing )");
      return ok;
    }
    if (["AND", "OR", "WITH", ")"].includes(token.toUpperCase())) {
      throw new Error(`unexpected ${token}`);
    }
    if (peek() === "WITH") {
      pos++;
      const exception = tokens[pos++];
      if (exception === undefined) throw new Error("missing exception");
      return allow.has(normalize(`${token} WITH ${exception}`));
    }
    return allow.has(normalize(token));
  }

  try {
    const ok = parseOr();
    return pos === tokens.length && ok;
  } catch {
    return false;
  }
}

/** Returns `{ name, version, license }` for every lockfile package that fails. */
export function findViolations(lock, allow) {
  if (!lock.packages) {
    throw new Error("package-lock.json has no `packages` (lockfileVersion 2+ required)");
  }
  const violations = new Map(); // one entry per name@version, however often it's nested
  for (const [path, pkg] of Object.entries(lock.packages)) {
    if (path === "" || pkg.link) continue; // the app itself, workspace links
    if (!isAllowed(pkg.license, allow)) {
      const name = pkg.name ?? path.replace(/^.*node_modules\//, "");
      const license = pkg.license ?? "(none)";
      violations.set(`${name}@${pkg.version}`, { name, version: pkg.version, license });
    }
  }
  return [...violations.values()];
}

function main([lockPath = "package-lock.json", denyPath = "deny.toml"]) {
  const allow = parseAllowList(readFileSync(denyPath, "utf8"));
  const lock = JSON.parse(readFileSync(lockPath, "utf8"));
  const count = Object.keys(lock.packages ?? {}).length - 1;
  const violations = findViolations(lock, allow);
  if (violations.length > 0) {
    console.error(`npm licenses not allowed by ${denyPath}:`);
    for (const v of violations) console.error(`  ${v.name}@${v.version}: ${v.license}`);
    process.exit(1);
  }
  console.log(`npm licenses ok: ${count} packages checked against ${denyPath}`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
