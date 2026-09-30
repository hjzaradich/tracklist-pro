import { KEY_NAMES, type KeyNotation } from "../bindings";

// Formats keys in the notation the user chose (ROADMAP §1.1 "Key notation").
// Keys are stored as Camelot (tags/key.rs); the names for every notation come
// from the backend's own tables, exported into the generated bindings as
// KEY_NAMES, so the frontend and backend can't spell a key differently.

export type { KeyNotation };

/** Camelot until the user picks another notation. */
export const DEFAULT_KEY_NOTATION: KeyNotation = "camelot";

/** Every notation, in the order a settings screen would list them. */
export const KEY_NOTATIONS: readonly KeyNotation[] = KEY_NAMES.map((table) => table.notation);

const NAMES = new Map<string, readonly string[]>(KEY_NAMES.map((table) => [table.notation, table.names]));

/** `8A`, `08A`, `12b`: a stored Camelot key. */
const CAMELOT = /^\s*0?([1-9]|1[0-2])\s*([AB])\s*$/i;

/** Where a Camelot key sits in the KEY_NAMES tables (1A–12A, then 1B–12B), or -1. */
function camelotIndex(camelot: string): number {
  const match = CAMELOT.exec(camelot);
  if (!match) return -1;
  const offset = match[2].toUpperCase() === "A" ? 0 : 12;
  return offset + Number(match[1]) - 1;
}

/**
 * A stored Camelot key in `notation`: formatKey("8A", "musical_standard") is
 * "Am". Returns null for no key, or for anything that isn't a Camelot key.
 * A notation this build doesn't know (e.g. from a newer backend) shows Camelot.
 */
export function formatKey(camelot: string | null | undefined, notation: KeyNotation): string | null {
  if (camelot == null) return null;
  const index = camelotIndex(camelot);
  if (index < 0) return null;
  const names = NAMES.get(notation) ?? NAMES.get(DEFAULT_KEY_NOTATION);
  return names?.[index] ?? null;
}
