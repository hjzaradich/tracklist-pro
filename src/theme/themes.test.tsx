import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import { DEFAULT_THEME, syncThemeToDocument, useThemeStore, type Theme } from "./themeStore";
import { tx } from "../test/tx";
import "./tokens.css";

// A sample screen (Overview, inside the app shell) rendered in both themes,
// with the styles jsdom applies to it: which color tokens the screen's own
// rules use, and what each token resolves to on the element that uses it.

type TokenUse = { element: Element; token: string };

function styleRules(rules: CSSRuleList): CSSStyleRule[] {
  return [...rules].flatMap((rule) => {
    if (rule instanceof CSSStyleRule) return [rule];
    // @media, @supports and similar group rules hold rules of their own.
    return "cssRules" in rule ? styleRules((rule as CSSGroupingRule).cssRules) : [];
  });
}

function matches(element: Element, selector: string): boolean {
  try {
    return element.matches(selector);
  } catch {
    return false; // a selector jsdom can't evaluate
  }
}

/**
 * Every var(--color-…) in the style rules that apply to the rendered screen,
 * with the element each applies to. Read from the declared rules because
 * jsdom's computed style drops shorthands like `border: 1px solid var(…)`.
 */
function colorTokenUses(root: Element): TokenUse[] {
  const rules = [...document.styleSheets].flatMap((sheet) => styleRules(sheet.cssRules));
  const uses: TokenUse[] = [];
  for (const element of [document.body, root, ...root.querySelectorAll("*")]) {
    for (const rule of rules.filter((r) => matches(element, r.selectorText))) {
      for (const property of rule.style) {
        const value = rule.style.getPropertyValue(property);
        for (const [, token] of value.matchAll(/var\(\s*(--color-[\w-]+)/g)) {
          uses.push({ element, token });
        }
      }
    }
  }
  return uses;
}

/** token → the value it resolves to where the screen uses it. */
function resolvedTokens(uses: TokenUse[]): Map<string, string> {
  return new Map(
    uses.map(({ element, token }) => [token, getComputedStyle(element).getPropertyValue(token).trim()]),
  );
}

describe("a sample screen in both themes", () => {
  let unsync: () => void;

  beforeEach(() => {
    useThemeStore.setState({ theme: DEFAULT_THEME });
    unsync = syncThemeToDocument();
  });

  afterEach(() => unsync());

  async function renderIn(theme: Theme) {
    const user = userEvent.setup();
    const { container } = renderApp("/overview");
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") });
    // Switched the way a user does it, with the theme switch in the top bar.
    await user.click(screen.getByRole("radio", { name: theme === "dark" ? tx("theme:dark") : tx("theme:light") }));
    return { container, user };
  }

  it("switches the theme attribute on <html> from dark to light and back", async () => {
    const { user } = await renderIn("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");

    await user.click(screen.getByRole("radio", { name: tx("theme:light") }));
    expect(document.documentElement.dataset.theme).toBe("light");

    await user.click(screen.getByRole("radio", { name: tx("theme:dark") }));
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("colors the screen with theme tokens (surfaces, borders, text, accent)", async () => {
    const { container } = await renderIn("dark");
    const used = new Set(colorTokenUses(container.firstElementChild!).map((u) => u.token));
    expect([...used]).toEqual(
      expect.arrayContaining([
        "--color-bg",
        "--color-text",
        "--color-text-muted",
        "--color-surface-raised",
        "--color-border",
        "--color-accent",
      ]),
    );
  });

  it("every color token the screen uses has a real value in each theme, and it differs between them", async () => {
    const { container, user } = await renderIn("dark");
    const uses = colorTokenUses(container.firstElementChild!);
    const dark = resolvedTokens(uses);

    await user.click(screen.getByRole("radio", { name: tx("theme:light") }));
    const light = resolvedTokens(uses);

    expect(dark.size).toBeGreaterThan(5);
    for (const [token, darkValue] of dark) {
      const lightValue = light.get(token);
      expect(darkValue, `${token} in dark`).toMatch(/^#[0-9a-f]{6}$/i);
      expect(lightValue, `${token} in light`).toMatch(/^#[0-9a-f]{6}$/i);
      expect(lightValue, `${token} is the same in both themes`).not.toBe(darkValue);
    }

    // And switching back restores the dark values exactly.
    await user.click(screen.getByRole("radio", { name: tx("theme:dark") }));
    expect(resolvedTokens(uses)).toEqual(dark);
  });
});
