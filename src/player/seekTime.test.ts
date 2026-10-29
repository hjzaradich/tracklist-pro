import { describe, expect, it } from "vitest";
import { ARROW_STEP, PAGE_STEP, timeAfterKey, timeAtPointer } from "./seekTime";

const BOX = { left: 100, width: 400 };

describe("the time a pointer on the waveform points at", () => {
  it("is the start at the left edge, the end at the right edge and the same share of the file between", () => {
    expect(timeAtPointer(100, BOX, 300)).toBe(0);
    expect(timeAtPointer(500, BOX, 300)).toBe(300);
    expect(timeAtPointer(300, BOX, 300)).toBe(150);
    expect(timeAtPointer(200, BOX, 300)).toBe(75);
  });

  it("holds a pointer beyond either edge to that edge, as a drag that leaves the bar does", () => {
    expect(timeAtPointer(-50, BOX, 300)).toBe(0);
    expect(timeAtPointer(99, BOX, 300)).toBe(0);
    expect(timeAtPointer(900, BOX, 300)).toBe(300);
  });

  it("has nowhere to point at when the bar has no width or the file no length", () => {
    expect(timeAtPointer(300, { left: 100, width: 0 }, 300)).toBe(0);
    expect(timeAtPointer(300, BOX, 0)).toBe(0);
    expect(timeAtPointer(300, BOX, Number.NaN)).toBe(0);
  });
});

describe("where the keys on the seek bar go", () => {
  it("moves a few seconds with the arrows, either way, and more with Page Up and Page Down", () => {
    expect(timeAfterKey("ArrowRight", 60, 300)).toBe(60 + ARROW_STEP);
    expect(timeAfterKey("ArrowUp", 60, 300)).toBe(60 + ARROW_STEP);
    expect(timeAfterKey("ArrowLeft", 60, 300)).toBe(60 - ARROW_STEP);
    expect(timeAfterKey("ArrowDown", 60, 300)).toBe(60 - ARROW_STEP);
    expect(timeAfterKey("PageUp", 60, 300)).toBe(60 + PAGE_STEP);
    expect(timeAfterKey("PageDown", 100, 300)).toBe(100 - PAGE_STEP);
  });

  it("goes to the ends with Home and End, and never past them", () => {
    expect(timeAfterKey("Home", 60, 300)).toBe(0);
    expect(timeAfterKey("End", 60, 300)).toBe(300);
    expect(timeAfterKey("ArrowLeft", 2, 300)).toBe(0);
    expect(timeAfterKey("PageUp", 290, 300)).toBe(300);
  });

  it("leaves every other key alone", () => {
    for (const key of ["a", "j", "k", " ", "Enter", "1", "Escape", "Tab"]) {
      expect(timeAfterKey(key, 60, 300), key).toBeNull();
    }
  });
});
