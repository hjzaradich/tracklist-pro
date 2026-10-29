import { describe, expect, it } from "vitest";
import { formatTime } from "./formatTime";

describe("the clock on the player bar", () => {
  it("writes minutes and two-digit seconds below an hour", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(5)).toBe("0:05");
    expect(formatTime(59)).toBe("0:59");
    expect(formatTime(60)).toBe("1:00");
    expect(formatTime(754)).toBe("12:34");
    expect(formatTime(3599)).toBe("59:59");
  });

  it("writes hours from an hour on, with two-digit minutes and seconds", () => {
    expect(formatTime(3600)).toBe("1:00:00");
    expect(formatTime(3661)).toBe("1:01:01");
    expect(formatTime(36_000)).toBe("10:00:00");
  });

  it("drops the fraction, never rounding up into the next second", () => {
    expect(formatTime(59.99)).toBe("0:59");
    expect(formatTime(3599.9)).toBe("59:59");
  });

  it("shows 0:00 for a time that isn't a time yet: not known, below zero, or endless", () => {
    expect(formatTime(Number.NaN)).toBe("0:00");
    expect(formatTime(-1)).toBe("0:00");
    expect(formatTime(Number.POSITIVE_INFINITY)).toBe("0:00");
  });
});
