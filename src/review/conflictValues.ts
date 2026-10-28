import type { ValueKind } from "../bindings";

/**
 * rekordbox's six ratings as it writes them in the export: the index is the
 * number of stars. The backend's check of a value given by hand
 * (`Track::stars`, `conflicts::fits`) accepts exactly these.
 */
export const RATING_VALUES = ["0", "51", "102", "153", "204", "255"] as const;

/** The stars a rating value stands for (an empty value is no rating, 0 stars); `null` if it is not one of the six. */
export function starsOf(value: string | null): number | null {
  if (value === null || value.trim() === "") return 0;
  const at = RATING_VALUES.indexOf(value as (typeof RATING_VALUES)[number]);
  return at < 0 ? null : at;
}

/** `0xRRGGBB` as a CSS color (`#rrggbb`); `null` if it is not one. */
export function colourOf(value: string | null): string | null {
  const match = /^0x([0-9a-f]{1,6})$/i.exec(value ?? "");
  return match ? `#${match[1]!.padStart(6, "0").toLowerCase()}` : null;
}

/** A CSS color (`#rrggbb`) as rekordbox writes it (`0xRRGGBB`). */
export function colourValue(css: string): string {
  return `0x${css.replace("#", "").toUpperCase()}`;
}

/** `2026-09-28` as a date in the user's language; the text itself if it is not one. */
export function dayOf(value: string, language: string): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  const date = new Date(`${value}T00:00:00Z`);
  // A day has no time zone: read it as UTC, show it as UTC.
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleDateString(language, { timeZone: "UTC" });
}

/** The fields where rekordbox writes 0 for "none" (a play count of 0 is a real count). */
const ZERO_IS_NONE = ["DiscNumber", "TrackNumber", "Year"];

/** Whether a value says "no value": an empty one, or 0 in a field where 0 means none. */
export function isNone(field: string, value: string | null): boolean {
  const text = (value ?? "").trim();
  return text === "" || (ZERO_IS_NONE.includes(field) && /^0+$/.test(text));
}

/** A file location as the user reads it: without the percent-encoding rekordbox adds. */
export function readableLocation(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/**
 * Whether `value` can be given by hand to a field of `kind`, so "Save" is
 * greyed out until it can. The backend decides in the end (`conflicts::fits`)
 * and refuses what this lets through (a date that is not on the calendar).
 * An empty value clears a colour (which has its own "No color" choice) and a
 * number. A date must be whole: a date field reads "" while it is half
 * typed, and would clear the field by accident. A rating is one of the six.
 * A file location is never given by hand: the app holds no value of its own
 * for it.
 */
export function canGive(kind: ValueKind, value: string): boolean {
  switch (kind) {
    case "rating":
      return (RATING_VALUES as readonly string[]).includes(value);
    case "colour":
      return value === "" || colourOf(value) !== null;
    case "date":
      return /^\d{4}-\d{2}-\d{2}$/.test(value);
    case "number":
      return value === "" || /^\d+$/.test(value);
    case "location":
    case "membership":
      return false;
    case "text":
      return true;
  }
}

/** What the edit starts from: the app's value, made into one the field can hold. */
export function startingValue(kind: ValueKind, value: string | null): string {
  const text = value ?? "";
  switch (kind) {
    case "rating":
      return canGive(kind, text) ? text : "0";
    default:
      return text;
  }
}
