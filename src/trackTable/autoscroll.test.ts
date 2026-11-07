import { describe, expect, it } from "vitest";
import { EDGE_ZONE, MAX_SCROLL_STEP, edgeScrollStep } from "./autoscroll";

// A list from y=100 to y=500.
const step = (y: number, inset = 0) => edgeScrollStep(y, 100, 500, { inset });

describe("the edge zone of a dragged list", () => {
  it("stays still in the middle of the list", () => {
    expect(step(300)).toBe(0);
    expect(step(100 + EDGE_ZONE)).toBe(0);
    expect(step(500 - EDGE_ZONE)).toBe(0);
  });

  it("scrolls up near the top and down near the bottom", () => {
    expect(step(120)).toBeLessThan(0);
    expect(step(480)).toBeGreaterThan(0);
  });

  it("scrolls faster the nearer the edge", () => {
    expect(Math.abs(step(110))).toBeGreaterThan(Math.abs(step(130)));
    expect(step(490)).toBeGreaterThan(step(470));
  });

  it("scrolls at the top speed at the edge itself and never faster", () => {
    expect(step(100)).toBe(-MAX_SCROLL_STEP);
    expect(step(500)).toBe(MAX_SCROLL_STEP);
  });

  it("scrolls at least a pixel anywhere in the zone", () => {
    expect(step(100 + EDGE_ZONE - 1)).toBe(-1);
    expect(step(500 - EDGE_ZONE + 1)).toBe(1);
  });

  it("does not scroll when the pointer is outside the list", () => {
    expect(step(99)).toBe(0);
    expect(step(501)).toBe(0);
  });

  it("measures the top zone from below a header that stays in view", () => {
    // The header takes 30 px: the zone starts under it and the header itself is the top edge.
    expect(step(100, 30)).toBe(-MAX_SCROLL_STEP);
    expect(step(130, 30)).toBe(-MAX_SCROLL_STEP);
    expect(step(130 + EDGE_ZONE, 30)).toBe(0);
  });

  it("leaves a still middle in a short list", () => {
    expect(edgeScrollStep(150, 100, 200)).toBe(0);
    expect(edgeScrollStep(110, 100, 200)).toBeLessThan(0);
    expect(edgeScrollStep(190, 100, 200)).toBeGreaterThan(0);
  });
});
