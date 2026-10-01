// Tests for copy-lib.mjs and copy.mjs. Run with: node --test scripts/
// The command tests build throwaway locale and notes folders.

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { approve, checkCopy, counts, flatten, placeholders, proposedStrings, readCopy } from "./copy-lib.mjs";
import { run } from "./copy.mjs";

const row = (approved, extra = {}) => ({ purpose: "What it's for.", where: "Where it shows.", ...(approved === undefined ? {} : { approved }), ...extra });

/** A project folder with these locale files and notes files ({ namespace: object }). */
function project(locales, notes) {
  const root = mkdtempSync(join(tmpdir(), "tlp-copy-"));
  mkdirSync(join(root, "src/locales/en"), { recursive: true });
  mkdirSync(join(root, "src/locales/notes"), { recursive: true });
  for (const [ns, value] of Object.entries(locales)) {
    writeFileSync(join(root, "src/locales/en", `${ns}.json`), JSON.stringify(value));
  }
  for (const [ns, value] of Object.entries(notes)) {
    writeFileSync(join(root, "src/locales/notes", `${ns}.json`), JSON.stringify(value));
  }
  return root;
}

function inProject(locales, notes, body) {
  const root = project(locales, notes);
  try {
    return body(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const notesOf = (root, ns) => JSON.parse(readFileSync(join(root, "src/locales/notes", `${ns}.json`), "utf8"));

const LOCALES = { demo: { title: "Title", remove: { done: "Removed {{title}}", conflicts_one: "{{count}} conflict", conflicts_other: "{{count}} conflicts" } } };
const APPROVED_NOTES = {
  demo: {
    title: row("Title"),
    "remove.done": row("Removed {{title}}"),
    "remove.conflicts_one": row("{{count}} conflict"),
    "remove.conflicts_other": row("{{count}} conflicts"),
  },
};
const problemsOf = (locales, notes) => inProject(locales, notes, (root) => checkCopy(readCopy(root)));

test("flatten gives every leaf a dotted key and refuses a value that isn't text", () => {
  assert.deepEqual([...flatten({ a: "x", b: { c: "y", d: { e: "z" } } })], [["a", "x"], ["b.c", "y"], ["b.d.e", "z"]]);
  assert.throws(() => flatten({ a: 3 }), /a: a locale value must be text/);
});

test("placeholders lists names, whatever the format or the count", () => {
  assert.deepEqual([...placeholders("{{count, number}} of {{ total }} and {{- raw}}")].sort(), ["count", "raw", "total"]);
  assert.equal(placeholders("no placeholders").size, 0);
});

test("a complete set of notes passes the check", () => {
  assert.deepEqual(problemsOf(LOCALES, APPROVED_NOTES), []);
});

test("a locale key with no notes row fails the check", () => {
  const notes = structuredClone(APPROVED_NOTES);
  delete notes.demo.title;
  assert.deepEqual(problemsOf(LOCALES, notes), ["demo:title: no notes row"]);
});

test("a namespace with no notes file fails the check", () => {
  const problems = problemsOf(LOCALES, {});
  assert.equal(problems.length, 1);
  assert.match(problems[0], /^demo: no notes file/);
});

test("a notes row whose locale key is gone fails the check", () => {
  const notes = structuredClone(APPROVED_NOTES);
  notes.demo.gone = row("Gone");
  assert.deepEqual(problemsOf(LOCALES, notes), ["demo:gone: notes row without a locale key"]);
});

test("a notes file whose locale file is gone fails the check", () => {
  const problems = problemsOf({}, APPROVED_NOTES);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /^demo: notes file .* has no locale file/);
});

test("an empty or missing purpose or where fails the check", () => {
  const notes = structuredClone(APPROVED_NOTES);
  notes.demo.title = { purpose: "  ", approved: "Title" };
  assert.deepEqual(problemsOf(LOCALES, notes), ['demo:title: "purpose" is empty', 'demo:title: "where" is empty']);
});

test("an unknown field or a non-text approved fails the check", () => {
  const notes = structuredClone(APPROVED_NOTES);
  notes.demo.title = row("Title", { note: "x" });
  notes.demo["remove.done"] = row(5);
  assert.deepEqual(problemsOf(LOCALES, notes), ['demo:title: unknown field "note"', 'demo:remove.done: "approved" must be text']);
});

test("a plural key without its pair fails the check", () => {
  const locales = { demo: { a_one: "{{count}} a" } };
  const notes = { demo: { a_one: row() } };
  assert.deepEqual(problemsOf(locales, notes), ["demo:a_one: no matching a_other"]);
  const other = { demo: { b_other: "{{count}} b" } };
  assert.deepEqual(problemsOf(other, { demo: { b_other: row() } }), ["demo:b_other: no matching b_one"]);
});

test("plural forms that use different placeholders fail the check", () => {
  const locales = { demo: { a_one: "{{count}} track by {{artist}}", a_other: "{{count}} tracks by {{name}}" } };
  const notes = { demo: { a_one: row(), a_other: row() } };
  const problems = problemsOf(locales, notes);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /demo: a: the _one and _other forms use different \{\{placeholders\}\}/);
});

test("the _one form may leave out {{count}}", () => {
  const locales = { demo: { a_one: "One track", a_other: "{{count}} tracks" } };
  assert.deepEqual(problemsOf(locales, { demo: { a_one: row(), a_other: row() } }), []);
});

test("plural forms that disagree in the approved text fail the check", () => {
  const locales = { demo: { a_one: "{{count}} a {{x}}", a_other: "{{count}} as {{x}}" } };
  const notes = { demo: { a_one: row("{{count}} a {{x}}"), a_other: row("{{count}} as {{y}}") } };
  const problems = problemsOf(locales, notes);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /demo: approved a: the _one and _other forms use different/);
});

test("a stray brace in the text fails the check", () => {
  const problems = problemsOf({ demo: { a: "Hello {{name" } }, { demo: { a: row() } });
  assert.deepEqual(problems, ['demo: a: has a "{{" or "}}" that isn\'t a whole {{placeholder}}']);
});

test("a string is approved while it equals the approved text", () => {
  inProject(LOCALES, APPROVED_NOTES, (root) => {
    assert.deepEqual(proposedStrings(readCopy(root)), []);
    assert.deepEqual(counts(readCopy(root)), { total: 4, proposed: 0 });
  });
});

test("a string with no approved text is proposed as new", () => {
  const notes = structuredClone(APPROVED_NOTES);
  delete notes.demo.title.approved;
  inProject(LOCALES, notes, (root) => {
    const [only, ...rest] = proposedStrings(readCopy(root));
    assert.equal(rest.length, 0);
    assert.deepEqual(only, { namespace: "demo", key: "title", text: "Title", purpose: "What it's for.", where: "Where it shows.", approved: undefined, kind: "new" });
  });
});

test("editing an approved string makes it proposed again", () => {
  const edited = structuredClone(LOCALES);
  edited.demo.title = "A new title";
  inProject(edited, APPROVED_NOTES, (root) => {
    const [only, ...rest] = proposedStrings(readCopy(root));
    assert.equal(rest.length, 0);
    assert.equal(only.key, "title");
    assert.equal(only.kind, "changed");
    assert.equal(only.text, "A new title");
    assert.equal(only.approved, "Title");
  });
});

test("a key with no notes row counts as proposed", () => {
  inProject(LOCALES, { demo: {} }, (root) => {
    assert.equal(proposedStrings(readCopy(root)).length, 4);
  });
});

test("approve sets approved to the current text of the named strings only", () => {
  const edited = structuredClone(LOCALES);
  edited.demo.title = "A new title";
  edited.demo.remove.done = "Gone {{title}}";
  inProject(edited, APPROVED_NOTES, (root) => {
    const { code, out } = run(["approve", "demo:title"], root);
    assert.equal(code, 0);
    assert.deepEqual(out, ["Approved 1 string."]);
    const notes = notesOf(root, "demo");
    assert.equal(notes.title.approved, "A new title");
    assert.equal(notes["remove.done"].approved, "Removed {{title}}");
    assert.deepEqual(proposedStrings(readCopy(root)).map((p) => p.key), ["remove.done"]);
  });
});

test("approve --all-in approves a whole namespace", () => {
  const edited = structuredClone(LOCALES);
  edited.demo.title = "A new title";
  edited.demo.remove.done = "Gone {{title}}";
  inProject(edited, { demo: { ...APPROVED_NOTES.demo, "remove.conflicts_one": row() } }, (root) => {
    const { code, out } = run(["approve", "--all-in", "demo"], root);
    assert.equal(code, 0);
    assert.deepEqual(out, ["Approved 3 strings."]);
    assert.deepEqual(proposedStrings(readCopy(root)), []);
    assert.equal(notesOf(root, "demo")["remove.conflicts_one"].approved, "{{count}} conflict");
  });
});

test("approve keeps purpose and where and the row order", () => {
  const edited = structuredClone(LOCALES);
  edited.demo.title = "A new title";
  inProject(edited, APPROVED_NOTES, (root) => {
    run(["approve", "demo:title"], root);
    const notes = notesOf(root, "demo");
    assert.deepEqual(Object.keys(notes), Object.keys(APPROVED_NOTES.demo));
    assert.deepEqual(notes.title, { purpose: "What it's for.", where: "Where it shows.", approved: "A new title" });
  });
});

test("approve refuses an unknown namespace or key, and changes nothing", () => {
  const edited = structuredClone(LOCALES);
  edited.demo.title = "A new title";
  inProject(edited, APPROVED_NOTES, (root) => {
    const result = run(["approve", "demo:title", "demo:nope", "other:title", "demo", "--all-in", "ghost"], root);
    assert.equal(result.code, 1);
    assert.deepEqual(result.err, [
      "demo:nope: no such key",
      "other:title: no such namespace",
      "demo: write it as <namespace>:<key>",
      "ghost: no such namespace",
      "Nothing was approved.",
    ]);
    assert.equal(notesOf(root, "demo").title.approved, "Title");
  });
});

test("approve refuses a string with no notes row", () => {
  const result = approve({ demo: { values: new Map([["a", "A"]]), notes: {} } }, ["demo:a"]);
  assert.deepEqual(result.problems, ["demo:a: no notes row to approve (run npm run copy:check)"]);
  assert.equal(result.approved, 0);
});

test("approve with nothing named, or --all-in with no namespace, is a usage error", () => {
  inProject(LOCALES, APPROVED_NOTES, (root) => {
    assert.equal(run(["approve"], root).code, 2);
    assert.equal(run(["approve", "--all-in"], root).code, 2);
  });
});

test("check exits 0 when the notes match and 1 with the problems when they don't", () => {
  inProject(LOCALES, APPROVED_NOTES, (root) => {
    assert.equal(run(["check"], root).code, 0);
  });
  const notes = structuredClone(APPROVED_NOTES);
  delete notes.demo.title;
  inProject(LOCALES, notes, (root) => {
    const { code, err } = run(["check"], root);
    assert.equal(code, 1);
    assert.equal(err[0], "demo:title: no notes row");
  });
});

test("status lists the proposed strings and exits 0 whatever is proposed", () => {
  const edited = structuredClone(LOCALES);
  edited.demo.title = "A new title";
  inProject(edited, APPROVED_NOTES, (root) => {
    const { code, out } = run(["status"], root);
    assert.equal(code, 0);
    assert.match(out[0], /demo:title {2}\[changed\]/);
    assert.match(out[0], /text: {4}"A new title"/);
    assert.match(out[0], /was: {5}"Title"/);
    assert.match(out[0], /purpose: What it's for\./);
    assert.match(out[0], /where: {3}Where it shows\./);
    assert.match(out[0], /1 of 4 strings proposed$/);
  });
  inProject(LOCALES, APPROVED_NOTES, (root) => {
    const { code, out } = run(["status"], root);
    assert.equal(code, 0);
    assert.equal(out[0], "0 of 4 strings proposed");
  });
});

test("gate exits 0 when everything is approved and 1 when anything is proposed", () => {
  inProject(LOCALES, APPROVED_NOTES, (root) => {
    assert.equal(run(["gate"], root).code, 0);
  });
  const notes = structuredClone(APPROVED_NOTES);
  delete notes.demo.title.approved;
  inProject(LOCALES, notes, (root) => {
    const { code, err } = run(["gate"], root);
    assert.equal(code, 1);
    assert.match(err.join("\n"), /strings are still proposed/);
  });
});

test("gate also fails when the notes are broken, even if nothing reads as proposed", () => {
  inProject(LOCALES, { demo: { ...APPROVED_NOTES.demo, extra: row("x") } }, (root) => {
    assert.equal(run(["gate"], root).code, 1);
    assert.equal(run(["status"], root).code, 0);
  });
});

test("an unknown command is a usage error", () => {
  inProject(LOCALES, APPROVED_NOTES, (root) => {
    assert.equal(run(["frobnicate"], root).code, 2);
    assert.equal(run([], root).code, 2);
  });
});

test("the real notes cover the real locale files", () => {
  assert.deepEqual(checkCopy(readCopy()), []);
});
