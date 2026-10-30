import { describe, expect, it } from "vitest";
import css from "./TrackTable.module.css?raw";

// jsdom has no layout, so the pinned column is held by its stylesheet: the
// first column sticks to the left edge, and in every state of a row it has
// that row's own background (a cell with none lets the columns slide under it).

/** Every declaration of every rule that names `selector`, in order: the later one wins. */
function rule(selector: string): string {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const found = [...stripped.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter((m) => m[1].split(",").some((part) => part.trim() === selector))
    .map((m) => m[2]);
  if (found.length === 0) throw new Error(`no rule for ${selector}`);
  return found.join(" ");
}

/** The last `background` the body gives. */
const background = (body: string) => [...body.matchAll(/background:\s*(var\(--[\w-]+\))/g)].at(-1)?.[1];

describe("the pinned first column", () => {
  it("sticks to the left edge in the header and in every row", () => {
    for (const selector of [".headRow > :first-child", ".row > :first-child"]) {
      const body = rule(selector);
      expect(body, selector).toMatch(/position:\s*sticky/);
      expect(body, selector).toMatch(/left:\s*0/);
    }
  });

  it("has a line at its right edge, drawn as a shadow so it takes no room", () => {
    expect(rule(".row > :first-child")).toMatch(/box-shadow:\s*1px 0 0 var\(--color-border\)/);
  });

  it("has the row's own background in every state", () => {
    // The row's states, and the first cell's, side by side.
    const pairs: [string, string][] = [
      [".row", ".row > :first-child"],
      [".stripe", ".stripe > :first-child"],
      [".row:hover", ".row:hover > :first-child"],
      [".selected", ".selected > :first-child"],
    ];
    for (const [row, cell] of pairs) {
      const own = background(rule(row)) ?? "var(--color-surface)";
      expect(background(rule(cell)), cell).toBe(own);
    }
  });

  it("keeps the selected row's accent bar on the pinned cell, and its line", () => {
    const body = rule(".selected > :first-child");
    expect(body).toMatch(/inset 3px 0 0 var\(--color-accent\)/);
    expect(body).toMatch(/1px 0 0 var\(--color-border\)/);
  });

  it("is above the other cells of its row, and the header's above the rows'", () => {
    const z = (selector: string) => Number([...rule(selector).matchAll(/z-index:\s*(\d+)/g)].at(-1)?.[1]);
    expect(z(".row > :first-child")).toBeGreaterThan(0);
    expect(z(".headRow > :first-child")).toBeGreaterThan(z(".row > :first-child"));
  });

  it("has the header's background in the header", () => {
    expect(background(rule(".headRow > :first-child"))).toBe(background(rule(".header")));
  });
});
