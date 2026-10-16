import ts from "typescript";
import { describe, expect, it } from "vitest";

// Tests never pin the app's wording (1aF-3, docs/copy-style.md): the owner
// edits UI text on their own schedule, and a test that spells it out breaks
// every time. They look a text up with tx() (src/test/tx.ts) instead. This
// check finds a test that has copied a UI text: a string, template or regex
// that is a whole locale text, or holds a long run of one. It parses the
// tests, so comments and test titles don't count. `npm run test:markers` is
// the full proof (the suite runs with every text replaced); this is the quick
// one that points at the line.

/** A test may repeat this many characters of a text before it counts as a copy. */
const LONG_RUN = 14;
/** Shorter whole texts ("Add", "Done", "Artist") are also ordinary data, so only longer ones count here. */
const SHORTEST_WHOLE = 8;

/** Texts that name the app, not the UI: they're in paths and identifiers all over. */
const NOT_COPY = new Set(["common.appName"]);

type Messages = { [key: string]: string | Messages };

// The files as written, not the loaded resources: the marker run
// (npm run test:markers) swaps those.
const localeFiles = import.meta.glob<string>("../locales/en/*.json", {
  query: "?raw",
  import: "default",
  eager: true,
});

function leaves(messages: Messages, prefix: string, out: Map<string, string>) {
  for (const [name, value] of Object.entries(messages)) {
    const key = `${prefix}.${name}`;
    if (typeof value === "string") out.set(key, value);
    else leaves(value, key, out);
  }
  return out;
}

/** Every English text, by `namespace.key`, as written in the locale files. */
function localeTexts(files: Record<string, string>): Map<string, string> {
  const out = new Map<string, string>();
  for (const [path, messages] of Object.entries(files)) {
    const namespace = path.split("/").pop()!.replace(/\.json$/, "");
    leaves(JSON.parse(messages) as Messages, namespace, out);
  }
  return out;
}

const PLACEHOLDER = /\{\{[^}]*\}\}/g;

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * The locale texts, ready to be looked for in many strings. Made once per set
 * of texts: a test source has thousands of strings, and checking each against
 * every text (building a pattern each time, trying every window of every run)
 * made this check the slowest in the suite, and the one that timed out when the
 * machine was busy.
 */
interface Finder {
  keys: string[];
  /** Per text: its runs between placeholders, and the pattern that matches it whole (or `null` if it is too short to count). */
  wholes: ({ runs: string[]; pattern: RegExp } | null)[];
  /** Every LONG_RUN-character window of every run, with the first text that has it. */
  windows: Map<string, number>;
}

function finderFor(texts: Map<string, string>): Finder {
  const keys: string[] = [];
  const wholes: Finder["wholes"] = [];
  const windows = new Map<string, number>();
  for (const [key, value] of texts) {
    if (NOT_COPY.has(key)) continue;
    const index = keys.length;
    keys.push(key);
    const runs = value.split(PLACEHOLDER);
    wholes.push(
      value.length >= SHORTEST_WHOLE
        ? { runs, pattern: new RegExp(`^${runs.map(escapeRegExp).join(".*")}$`, "s") }
        : null,
    );
    for (const run of runs) {
      for (let at = 0; at + LONG_RUN <= run.length; at += 1) {
        const window = run.slice(at, at + LONG_RUN);
        if (!windows.has(window)) windows.set(window, index);
      }
    }
  }
  return { keys, wholes, windows };
}

const finders = new WeakMap<Map<string, string>, Finder>();

/** Whether `text` is a whole locale text (placeholders filled with anything) or holds a long run of one. */
function copiedFrom(text: string, texts: Map<string, string>): string | undefined {
  const trimmed = text.trim();
  if (trimmed === "") return undefined;
  let finder = finders.get(texts);
  if (finder === undefined) {
    finder = finderFor(texts);
    finders.set(texts, finder);
  }
  // The first text, in order, that is a copy: whole, or holding a long run.
  let first = Infinity;
  for (let index = 0; index < finder.wholes.length; index += 1) {
    const whole = finder.wholes[index];
    if (!whole) continue;
    const { runs, pattern } = whole;
    // Cheap checks before the pattern: both ends of the text are fixed runs.
    if (!trimmed.startsWith(runs[0]!) || !trimmed.endsWith(runs[runs.length - 1]!)) continue;
    if (pattern.test(trimmed)) {
      first = index;
      break;
    }
  }
  for (let at = 0; at + LONG_RUN <= text.length; at += 1) {
    const found = finder.windows.get(text.slice(at, at + LONG_RUN));
    if (found !== undefined && found < first) first = found;
  }
  return first === Infinity ? undefined : finder.keys[first];
}

const TITLE_CALLEES = new Set(["it", "test", "describe", "suite"]);

/** Whether `node` is the title argument of it(), describe() and the like, `.each` and `.skip` included. */
function isTestTitle(node: ts.Node): boolean {
  const call = node.parent;
  if (!ts.isCallExpression(call) || call.arguments[0] !== node) return false;
  let callee: ts.Expression = call.expression;
  // it.each([...])("title", ...)
  if (ts.isCallExpression(callee)) callee = callee.expression;
  while (ts.isPropertyAccessExpression(callee)) callee = callee.expression;
  return ts.isIdentifier(callee) && TITLE_CALLEES.has(callee.text);
}

/** `line: "text"` for every copied locale text in a test's source. */
function copiesIn(source: string, texts: Map<string, string>, path = "x.tsx"): string[] {
  const file = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found: string[] = [];
  const check = (node: ts.Node, text: string) => {
    const key = copiedFrom(text, texts);
    if (key === undefined) return;
    const { line } = file.getLineAndCharacterOfPosition(node.getStart(file));
    found.push(`${line + 1}: ${JSON.stringify(text)} (the text of ${key})`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node)) return;
    if (
      (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) &&
      !isTestTitle(node) &&
      !(ts.isPropertyAssignment(node.parent) && node.parent.name === node)
    ) {
      check(node, node.text);
    } else if (ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) {
      check(node, node.text);
    } else if (ts.isRegularExpressionLiteral(node)) {
      // The pattern's text, escapes removed: /Add\./ is looking for "Add.".
      check(node, node.text.replace(/^\/(.*)\/[a-z]*$/s, "$1").replace(/\\(.)/g, "$1"));
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe("copiesIn", () => {
  const texts = new Map([
    ["a.save", "Save these tracks"],
    ["a.long", "Your Library changed after the review."],
    ["a.named", "Remove {{title}} from your Library?"],
  ]);

  it("flags a string, template or regex that is a whole text", () => {
    const source = `
      expect(a).toHaveTextContent("Save these tracks");
      expect(b).toHaveTextContent(\`Save these tracks\`);
      expect(c).toHaveTextContent(/Save these tracks/);
    `;
    expect(copiesIn(source, texts).map((c) => c.split(":")[0])).toEqual(["2", "3", "4"]);
  });

  it("flags a text with its placeholders filled in", () => {
    expect(copiesIn('expect(a).toBe("Remove Song from your Library?");', texts)).toEqual([
      '1: "Remove Song from your Library?" (the text of a.named)',
    ]);
  });

  it("flags a long run of a text, in a regex too", () => {
    expect(copiesIn('findByText(/changed after the review/)', texts)).toHaveLength(1);
    expect(copiesIn('findByText("Oops. Your Library changed after the review. Try again.")', texts)).toHaveLength(1);
  });

  it("leaves data, test titles, keys and comments alone", () => {
    const source = `
      // Save, and "Your Library changed after the review."
      it("saves it", () => {});
      describe.each(["x"])("Your Library changed after the review.", () => {});
      it.each([1])("Save these tracks", () => {});
      const song = { title: "Song", Save: 1 };
      expect(a).toBe("saved");
      import "Save";
    `;
    expect(copiesIn(source, texts)).toEqual([]);
  });
});

describe("the tests hold no copy of the app's UI text", () => {
  const sources = import.meta.glob<string>(["../**/*.test.{ts,tsx}", "!./noCopyInTests.test.ts"], {
    query: "?raw",
    import: "default",
    eager: true,
  });
  const texts = localeTexts(localeFiles);

  it("finds the tests and the texts to check", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(10);
    expect(texts.size).toBeGreaterThan(100);
  });

  // The check parses every test file, so how long it takes grows with the
  // suite, and it is slowed most when the full run keeps every worker busy.
  // It is fast alone (about a second); 60 s is for a loaded machine, not a
  // budget to grow into.
  it("look a text up with tx() instead of spelling it out", { timeout: 60_000 }, () => {
    const offenders = Object.entries(sources).flatMap(([path, source]) =>
      copiesIn(source, texts, path).map((copy) => `${path}:${copy}`),
    );
    expect(offenders).toEqual([]);
  });
});
