import type { PaletteAction } from "./registry";

/** Words of a text: lowercase, accents folded, split at anything that isn't a letter or digit. */
function words(text: string): string[] {
  return text
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((word) => word !== "");
}

/**
 * Whether `query` finds `label`: every word typed starts a word of the label,
 * in any order and case ("lib" and "go lib" find "Go to Library"). Nothing
 * typed finds everything.
 */
export function matches(label: string, query: string): boolean {
  const have = words(label);
  return words(query).every((typed) => have.some((word) => word.startsWith(typed)));
}

/** The actions `query` finds, in the order given (no ranking). */
export function filterActions(actions: readonly PaletteAction[], query: string): PaletteAction[] {
  return actions.filter((action) => matches(action.label, query));
}
