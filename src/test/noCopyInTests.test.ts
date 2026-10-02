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

/** Whether `text` is a whole locale text (placeholders filled with anything) or holds a long run of one. */
function copiedFrom(text: string, texts: Map<string, string>): string | undefined {
  const trimmed = text.trim();
  if (trimmed === "") return undefined;
  for (const [key, value] of texts) {
    if (NOT_COPY.has(key)) continue;
    const runs = value.split(PLACEHOLDER);
    if (value.length >= SHORTEST_WHOLE) {
      const whole = new RegExp(`^${runs.map(escapeRegExp).join(".*")}$`, "s");
      if (whole.test(trimmed)) return key;
    }
    for (const run of runs) {
      for (let at = 0; at + LONG_RUN <= run.length; at += 1) {
        if (text.includes(run.slice(at, at + LONG_RUN))) return key;
      }
    }
  }
  return undefined;
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

  it("look a text up with tx() instead of spelling it out", () => {
    const offenders = Object.entries(sources).flatMap(([path, source]) =>
      copiesIn(source, texts, path).map((copy) => `${path}:${copy}`),
    );
    expect(offenders).toEqual([]);
  });
});
