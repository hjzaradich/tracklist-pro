import { describe, expect, it } from "vitest";
import { addedOn, isoTime } from "./ReviewComparison";

// SQLite writes a time as "YYYY-MM-DD HH:MM:SS", in UTC, with no zone. Read as a
// bare date it would be the viewer's local time, a day off for part of every
// day. The zone is a fact of the machine the test runs on, so the test holds
// the text that is handed to Date, which is the same everywhere.

describe("the date added", () => {
  it("reads a SQLite time as UTC", () => {
    expect(isoTime("2026-09-30 02:30:00")).toBe("2026-09-30T02:30:00Z");
    expect(new Date(isoTime("2026-09-30 02:30:00")).toISOString()).toBe("2026-09-30T02:30:00.000Z");
  });

  it("leaves an ISO time with its own zone as it is", () => {
    expect(isoTime("2026-09-30T02:30:00.000Z")).toBe("2026-09-30T02:30:00.000Z");
  });

  it("shows the viewer's own day for that moment", () => {
    const day = new Intl.DateTimeFormat("en", { dateStyle: "medium" }).format(
      new Date("2026-09-30T02:30:00Z"),
    );
    expect(addedOn("en", "2026-09-30 02:30:00")).toBe(day);
  });

  it("shows text that is not a date as it is, and nothing for no date", () => {
    expect(addedOn("en", "not a date")).toBe("not a date");
    expect(addedOn("en", null)).toBe("");
  });
});
