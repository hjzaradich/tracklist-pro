import { ESLint } from "eslint";
import { beforeAll, describe, expect, it } from "vitest";

// Lints snippets through the project's real eslint.config.js, so these tests
// prove both what the rule catches and that it's switched on for src/.
const RULE = "tracklist-pro/no-raw-jsx-strings";

let eslint: ESLint;
beforeAll(async () => {
  eslint = new ESLint();
  // The first lint loads the whole config and its plugins, which is slow;
  // do it here so no single test pays for it.
  await eslint.lintText("export {};", { filePath: "src/feature/Warmup.tsx" });
}, 120_000);

/** The rule's messages for a snippet placed at `filePath`. */
async function ruleMessages(code: string, filePath = "src/feature/Example.tsx") {
  const [result] = await eslint.lintText(code, { filePath });
  const fatal = result.messages.filter((m) => m.fatal);
  expect(fatal, "snippet failed to parse").toEqual([]);
  return result.messages.filter((m) => m.ruleId === RULE);
}

/** The raw strings the rule reports in a snippet placed at `filePath`. */
async function reported(code: string, filePath = "src/feature/Example.tsx"): Promise<string[]> {
  return (await ruleMessages(code, filePath)).map(
    (m) => /"([^"]*)"\. Add it/.exec(m.message)?.[1] ?? m.message,
  );
}

/**
 * A component returning `jsx`. `body` goes inside the component before the
 * return; `top` goes at module level.
 */
function component(jsx: string, body = "", top = ""): string {
  return `
    import { useTranslation } from "react-i18next";
    declare const cond: boolean;
    declare const name: string;
    declare const count: number;
    declare const rest: Record<string, unknown>;
    declare function EmptyState(props: Record<string, unknown>): null;
    ${top}
    export function Example() {
      const { t } = useTranslation("home");
      ${body}
      return (${jsx});
    }
  `;
}

describe("no raw strings in JSX (lint)", { timeout: 120_000 }, () => {
  it("flags plain text children: <p>Hello</p>", async () => {
    expect(await reported(component(`<p>Hello</p>`))).toEqual(["Hello"]);
  });

  it('flags aria-label="Close"', async () => {
    expect(await reported(component(`<button aria-label="Close" />`))).toEqual(["Close"]);
  });

  it("flags every user-visible attribute: title, alt, placeholder, aria text", async () => {
    const found = await reported(
      component(`<div>
        <img alt="Cover art" />
        <input title="Search" placeholder="Artist or title" />
        <div aria-description="Deck A" aria-roledescription="slider" aria-valuetext="Half" />
      </div>`),
    );
    expect(found).toEqual([
      "Cover art",
      "Search",
      "Artist or title",
      "Deck A",
      "slider",
      "Half",
    ]);
  });

  it("flags strings hidden in braces, templates, ?: and &&", async () => {
    const found = await reported(
      component(`<div title={"Tip"}>
        {"Hello"}
        {\`Hi \${name}\`}
        {cond ? "Yes" : "No"}
        {cond && "Shown"}
      </div>`),
    );
    expect(found).toEqual(["Tip", "Hello", "Hi …", "Yes", "No", "Shown"]);
  });

  it("flags text props on components (label, message, emptyText, ...)", async () => {
    const found = await reported(
      component(
        `<EmptyState label="Crates" message="No tracks yet" emptyText="Nothing here" ariaLabel="Empty" />`,
      ),
    );
    expect(found).toEqual(["Crates", "No tracks yet", "Nothing here", "Empty"]);
  });

  it("does not flag className, data-*, or other non-visible attributes", async () => {
    const found = await reported(
      component(`<button
        className="primary wide"
        data-testid="close button"
        data-label="Close"
        type="button"
        role="switch"
        id="closeButton"
        key="close"
        aria-labelledby="heading"
        context="library"
        i18nKey="home:title"
      />`),
    );
    expect(found).toEqual([]);
  });

  it("allows t() in children and attributes", async () => {
    const found = await reported(
      component(`<p title={t("title")} aria-label={cond ? t("title") : t("body")}>
        {t("body")}
      </p>`),
    );
    expect(found).toEqual([]);
  });

  it("allows text with no letters: numbers, punctuation, whitespace", async () => {
    const found = await reported(component(`<p>{t("title")} · 42 — {"%"} / {" "}</p>`));
    expect(found).toEqual([]);
  });

  it('flags string concatenation: {"Hi " + name}, in children and attributes', async () => {
    const found = await reported(
      component(`<p title={"Tip: " + name}>
        {"Hi " + name}
        {name + " tracks left"}
        {cond ? "Deck " + name : t("body")}
      </p>`),
    );
    expect(found).toEqual(["Tip:", "Hi", "tracks left", "Deck"]);
  });

  it("allows concatenation with no letters, and arithmetic", async () => {
    const found = await reported(
      component(`<p title={name + " · " + t("title")}>{count + 1}{name + " / "}{"#" + count}</p>`),
    );
    expect(found).toEqual([]);
  });

  it('flags a same-file const holding raw text: const msg = "Hello"; <p>{msg}</p>', async () => {
    const found = await reported(
      component(
        `<p aria-label={label}>{msg}{greeting}{joined}{alias}</p>`,
        [
          `const msg = "Hello";`,
          "const greeting = `Hi ${name}`;",
          `const joined = "Welcome " + name;`,
          `const label = cond ? "Open" : t("title");`,
        ].join("\n"),
        [`const MODULE_TEXT = "Library";`, `const alias = MODULE_TEXT;`].join("\n"),
      ),
    );
    expect(found).toEqual(["Open", "Hello", "Hi …", "Welcome", "Library"]);
  });

  it("follows a const every time one expression uses it, not just the first", async () => {
    const found = await reported(
      component(
        `<p title={label + " " + label}>{cond ? label : label}{deck + label}</p>`,
        [`const label = "Hi";`, `const deck = label + " there";`].join("\n"),
      ),
    );
    expect(found).toEqual(["Hi", "Hi", "Hi", "Hi", "Hi", "there", "Hi"]);
  });

  it("stops on a const that refers back to itself (a → b → a) and still reports its text", async () => {
    const found = await reported(
      component(`<p>{a}</p>`, [`const a = b + " left";`, `const b = a;`].join("\n")),
    );
    expect(found).toEqual(["left"]);
  });

  it("reports a const's text where it's used in JSX, not where it's declared", async () => {
    const code = component(`<p>{msg}</p>`, `const msg = "Hello";`);
    const messages = await ruleMessages(code);
    const jsxLine = code.split("\n").findIndex((line) => line.includes("{msg}")) + 1;
    expect(messages.map((m) => m.line)).toEqual([jsxLine]);
  });

  it("does not flag names bound to t(), let/var, parameters, imports, numbers or destructuring", async () => {
    const found = await reported(`
      import { useTranslation } from "react-i18next";
      import { APP_NAME } from "./constants";
      let draft = "Draft";
      var legacy = "Legacy";
      export function Example({ text }: { text: string }) {
        const { t } = useTranslation("home");
        const title = t("title");
        const both = t("title") + t("body");
        const total = 42;
        const [first] = ["Destructured"];
        return (
          <p title={title} aria-label={both}>
            {draft}{legacy}{text}{APP_NAME}{total}{first}
          </p>
        );
      }
    `);
    expect(found).toEqual([]);
  });

  it('flags text props in an object spread: <Comp {...{ label: "Hi" }} />', async () => {
    const found = await reported(
      component(
        '<EmptyState {...{ label: "Hi", title: `Tip`, "aria-label": "Close", emptyText: "Nothing" }} />',
      ),
    );
    expect(found).toEqual(["Hi", "Tip", "Close", "Nothing"]);
  });

  it("flags text props in a spread of a same-file const object", async () => {
    const found = await reported(
      component(`<EmptyState {...props} />`, `const props = { message: "No tracks", id: "empty" };`),
    );
    expect(found).toEqual(["No tracks"]);
  });

  it("does not flag non-text props, t() values, let objects or unknown objects in a spread", async () => {
    const found = await reported(
      component(
        `<div>
          <EmptyState {...{ className: "wide", "data-label": "Close", role: "note", id: "x", label: t("title") }} />
          <EmptyState {...rest} />
          <EmptyState {...editable} />
          <EmptyState {...{ ["label"]: name }} />
        </div>`,
        `let editable = { label: "Hi" };`,
      ),
    );
    expect(found).toEqual([]);
  });

  it("does not apply to test files, whose fixtures never reach a user", async () => {
    expect(await reported(component(`<p>Hello</p>`), "src/feature/Example.test.tsx")).toEqual(
      [],
    );
  });
});
