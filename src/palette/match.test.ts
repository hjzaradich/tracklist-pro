import { describe, expect, it } from "vitest";
import { filterActions, matches } from "./match";
import type { PaletteAction } from "./registry";

const action = (id: string, label: string): PaletteAction => ({ id, label, run: () => {} });

describe("matching a command", () => {
  it("finds a label by the start of any of its words, in any case", () => {
    expect(matches("Alpha Beta Gamma", "bet")).toBe(true);
    expect(matches("Alpha Beta Gamma", "GAM")).toBe(true);
    expect(matches("Alpha Beta Gamma", "eta")).toBe(false);
  });

  it("needs every word typed to start some word, in any order", () => {
    expect(matches("Alpha Beta Gamma", "gam al")).toBe(true);
    expect(matches("Alpha Beta Gamma", "al delta")).toBe(false);
  });

  it("ignores accents and punctuation, and finds everything when nothing is typed", () => {
    expect(matches("Café: Noir", "cafe")).toBe(true);
    expect(matches("Café: Noir", "  ")).toBe(true);
    expect(matches("Café: Noir", "")).toBe(true);
  });

  it("keeps the order the actions were given", () => {
    const list = [action("c", "Gamma One"), action("a", "Alpha One"), action("b", "Beta Two")];
    expect(filterActions(list, "one").map((found) => found.id)).toEqual(["c", "a"]);
  });
});
