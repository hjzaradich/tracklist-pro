import type { TableFilter } from "../bindings";

/**
 * The doors from the Overview to the Library (1cC-4, "Every number is a
 * door"): a number opens `/library?filter=…`, and the Library starts with
 * that filter chip on. The `filter` search value is a word, so the address
 * stays readable and a made-up one means nothing rather than an error.
 *
 * Only a filter the Overview has a number for has a word. The chip is the
 * Library's own (src/trackTable): it is taken off with its ×, works with the
 * other filters and the search, and isn't remembered, like every chip.
 */
const PLAIN_DOORS = [
  "lowQuality",
  "analyzedByRekordbox",
  "notAnalyzedByRekordbox",
  "inCrate",
  "inNoCrate",
  "noEnergy",
] as const;

type PlainDoor = (typeof PLAIN_DOORS)[number];

const ENERGY_PREFIX = "energy-";
/** Energy runs from 1 to 10 (`ENERGY_LEVELS` in the backend). */
const ENERGY_LEVELS = 10;

function isPlainDoor(word: string): word is PlainDoor {
  return (PLAIN_DOORS as readonly string[]).includes(word);
}

/** The word a filter is written as in the address; `null` for a filter no number opens. */
export function doorWord(filter: TableFilter): string | null {
  if (filter.kind === "energy") return `${ENERGY_PREFIX}${filter.level}`;
  return isPlainDoor(filter.kind) ? filter.kind : null;
}

/** The filter a word in the address stands for; `null` for anything else. */
export function filterOfDoor(word: unknown): TableFilter | null {
  if (typeof word !== "string") return null;
  if (isPlainDoor(word)) return { kind: word };
  if (word.startsWith(ENERGY_PREFIX)) {
    const level = Number(word.slice(ENERGY_PREFIX.length));
    if (Number.isInteger(level) && level >= 1 && level <= ENERGY_LEVELS) return { kind: "energy", level };
  }
  return null;
}

/** The Library's `filter` search value: a word that stands for a filter, else none. */
export function validateLibrarySearch(search: Record<string, unknown>): { filter?: string } {
  const filter = filterOfDoor(search.filter);
  return filter === null ? {} : { filter: search.filter as string };
}
