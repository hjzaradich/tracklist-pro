// Tests for npm-licenses.mjs. Run with: node --test .github/scripts/

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseAllowList, isAllowed, findViolations } from "./npm-licenses.mjs";

const allow = new Set(["mit", "apache-2.0", "apache-2.0 with llvm-exception", "gpl-3.0-or-later"]);

test("single allowed and disallowed ids", () => {
  assert.equal(isAllowed("MIT", allow), true);
  assert.equal(isAllowed("mit", allow), true);
  assert.equal(isAllowed("GPL-2.0-only", allow), false);
  assert.equal(isAllowed("SSPL-1.0", allow), false);
});

test("OR needs one allowed side, AND needs both", () => {
  assert.equal(isAllowed("(MIT OR GPL-2.0-only)", allow), true);
  assert.equal(isAllowed("GPL-2.0-only OR Apache-2.0", allow), true);
  assert.equal(isAllowed("MIT AND GPL-2.0-only", allow), false);
  assert.equal(isAllowed("MIT AND Apache-2.0", allow), true);
  assert.equal(isAllowed("(MIT OR SSPL-1.0) AND (Apache-2.0 OR BUSL-1.1)", allow), true);
  assert.equal(isAllowed("(MIT OR SSPL-1.0) AND BUSL-1.1", allow), false);
  // AND binds tighter than OR.
  assert.equal(isAllowed("SSPL-1.0 AND BUSL-1.1 OR MIT", allow), true);
  assert.equal(isAllowed("MIT OR SSPL-1.0 AND BUSL-1.1", allow), true);
});

test("WITH must be allowed as written", () => {
  assert.equal(isAllowed("Apache-2.0 WITH LLVM-exception", allow), true);
  assert.equal(isAllowed("MIT WITH Some-exception", allow), false);
});

test("missing or non-SPDX licenses are not allowed", () => {
  for (const bad of [undefined, null, "", "UNLICENSED", "SEE LICENSE IN LICENSE.md", "MIT OR", "(MIT", "MIT)"]) {
    assert.equal(isAllowed(bad, allow), false, `${bad}`);
  }
});

test("findViolations lists only failing packages and skips the root and links, once per name@version", () => {
  const lock = {
    lockfileVersion: 3,
    packages: {
      "": { name: "app", license: "SSPL-1.0" },
      "node_modules/ok": { version: "1.0.0", license: "MIT" },
      "node_modules/@scope/bad": { version: "2.0.0", license: "GPL-2.0-only" },
      "node_modules/a/node_modules/none": { version: "3.0.0" },
      "node_modules/b/node_modules/@scope/bad": { version: "2.0.0", license: "GPL-2.0-only" },
      "packages/linked": { link: true },
    },
  };
  assert.deepEqual(findViolations(lock, allow), [
    { name: "@scope/bad", version: "2.0.0", license: "GPL-2.0-only" },
    { name: "none", version: "3.0.0", license: "(none)" },
  ]);
});

test("parseAllowList reads only [licenses].allow and ignores comments", () => {
  const toml = `
[graph]
allow = ["NOT-THIS"]

[licenses]
# allow = ["COMMENTED-OUT"]
allow = [
  "MIT", # trailing comment
  "Apache-2.0 WITH LLVM-exception",
  # "GPL-2.0-only",
]
confidence-threshold = 0.9

[bans]
allow = ["NOR-THIS"]
`;
  assert.deepEqual([...parseAllowList(toml)], ["mit", "apache-2.0 with llvm-exception"]);
});

test("the repo's deny.toml allows GPL-3.0 and rejects GPL-incompatible licenses", () => {
  const real = parseAllowList(readFileSync(new URL("../../deny.toml", import.meta.url), "utf8"));
  for (const ok of ["MIT", "Apache-2.0", "BSD-3-Clause", "ISC", "MPL-2.0", "GPL-3.0-or-later"]) {
    assert.equal(isAllowed(ok, real), true, ok);
  }
  for (const bad of ["GPL-2.0-only", "SSPL-1.0", "BUSL-1.1", "CC-BY-NC-4.0", "EPL-2.0", "OpenSSL"]) {
    assert.equal(isAllowed(bad, real), false, bad);
  }
});
