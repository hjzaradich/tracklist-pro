import { describe, expect, it } from "vitest";
import {
  canGive,
  colourOf,
  colourValue,
  dayOf,
  isNone,
  RATING_VALUES,
  readableLocation,
  starsOf,
  startingValue,
} from "./conflictValues";

describe("a rating", () => {
  it("is read in stars from rekordbox's six values, and an empty one is no rating", () => {
    expect(RATING_VALUES.map(starsOf)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(starsOf(null)).toBe(0);
    expect(starsOf("")).toBe(0);
    expect(starsOf("100")).toBeNull();
  });

  it("can be given only as one of the six values", () => {
    expect(RATING_VALUES.every((value) => canGive("rating", value))).toBe(true);
    expect(canGive("rating", "100")).toBe(false);
    expect(canGive("rating", "")).toBe(false);
  });

  it("starts an edit from the app's rating, or no rating when that is not one", () => {
    expect(startingValue("rating", "153")).toBe("153");
    expect(startingValue("rating", null)).toBe("0");
    expect(startingValue("rating", "100")).toBe("0");
  });
});

describe("a colour", () => {
  it("is read from 0xRRGGBB and written back the way rekordbox writes it", () => {
    expect(colourOf("0xFF0000")).toBe("#ff0000");
    expect(colourOf("0x00FF")).toBe("#0000ff");
    expect(colourOf("red")).toBeNull();
    expect(colourOf(null)).toBeNull();
    expect(colourValue("#ff0000")).toBe("0xFF0000");
  });

  it("can be given as a colour or as none", () => {
    expect(canGive("colour", "0x0000FF")).toBe(true);
    expect(canGive("colour", "")).toBe(true);
    expect(canGive("colour", "blue")).toBe(false);
  });
});

describe("a date", () => {
  it("is shown in the user's language without moving to another day", () => {
    const shown = dayOf("2026-09-28", "en");
    expect(shown).toContain("28");
    expect(shown).toContain("2026");
    expect(dayOf("last week", "en")).toBe("last week");
  });

  it("can be given only as a whole date: a half-typed one reads empty and would clear the field", () => {
    expect(canGive("date", "2026-09-28")).toBe(true);
    expect(canGive("date", "")).toBe(false);
    expect(canGive("date", "28/09/2026")).toBe(false);
  });
});

describe("a number", () => {
  it("can be given as whole digits or as none", () => {
    expect(canGive("number", "12")).toBe(true);
    expect(canGive("number", "")).toBe(true);
    expect(canGive("number", "-1")).toBe(false);
    expect(canGive("number", "1.5")).toBe(false);
    expect(canGive("number", "twelve")).toBe(false);
  });
});

describe("text", () => {
  it("can be given as anything", () => {
    expect(canGive("text", "")).toBe(true);
    expect(canGive("text", "House")).toBe(true);
  });
});

describe("a file location", () => {
  it("is never given by hand", () => {
    expect(canGive("location", "file://localhost/E:/a.mp3")).toBe(false);
    expect(canGive("location", "")).toBe(false);
  });

  it("is read without the percent-encoding, or as it is when that cannot be undone", () => {
    expect(readableLocation("file://localhost/E:/a%20b.mp3")).toBe("file://localhost/E:/a b.mp3");
    expect(readableLocation("file://localhost/E:/100%.mp3")).toBe("file://localhost/E:/100%.mp3");
  });
});

describe("no value", () => {
  it("is an empty value, or 0 where rekordbox writes 0 for none, but never a play count of 0", () => {
    expect(isNone("Genre", "")).toBe(true);
    expect(isNone("Genre", null)).toBe(true);
    expect(isNone("Genre", "0")).toBe(false);
    for (const field of ["Year", "DiscNumber", "TrackNumber"]) expect(isNone(field, "0")).toBe(true);
    expect(isNone("Year", "1999")).toBe(false);
    expect(isNone("PlayCount", "0")).toBe(false);
  });
});
