import type { TFunction } from "i18next";
import type { TableFilter } from "../bindings";
import type { TableScreen } from "./columns";

/**
 * The filter chips above the track table (1cA-4; ROADMAP 1.11 "The track
 * table"). A chip is a {@link TableFilter}: what the app already knows about
 * a track. Free filters such as "BPM 120 to 126" are 1cB's.
 */
export type FilterKind = TableFilter["kind"];

/**
 * The filters each screen offers (owner decision, 2026-10-26: "pick per
 * screen what makes sense"):
 * - the Library: missing, in or not in rekordbox, a format, a quality
 *   problem, in a crate or in no crate, from a low-quality file, analyzed or
 *   not analyzed by rekordbox, with missing tags, with missing art. The energy filters are not on offer: they
 *   come only from the Overview's energy bars (1cC-4), one level at a time;
 * - All music: the same except the crate filters, which mean nothing for a
 *   track that isn't in the Library, and in or not in the Library instead;
 * - a crate: missing, in or not in rekordbox, a format, a quality problem
 *   (every track in it is in a crate, and in the Library).
 */
export const FILTERS_BY_SCREEN: Readonly<Record<TableScreen, readonly FilterKind[]>> = {
  library: [
    "missing",
    "inRekordbox",
    "notInRekordbox",
    "format",
    "qualityProblem",
    "inCrate",
    "inNoCrate",
    "lowQuality",
    "analyzedByRekordbox",
    "notAnalyzedByRekordbox",
    "missingTags",
    "missingArt",
  ],
  allMusic: [
    "missing",
    "inRekordbox",
    "notInRekordbox",
    "format",
    "qualityProblem",
    "inLibrary",
    "notInLibrary",
  ],
  crate: ["missing", "inRekordbox", "notInRekordbox", "format", "qualityProblem"],
};

/** Filters that say the opposite of each other: adding one takes the other off. */
const OPPOSITES: readonly (readonly [FilterKind, FilterKind])[] = [
  ["inRekordbox", "notInRekordbox"],
  ["inCrate", "inNoCrate"],
  ["inLibrary", "notInLibrary"],
  ["analyzedByRekordbox", "notAnalyzedByRekordbox"],
];

/** The energy filters: one at a time, a new one takes the old one off. */
const ENERGY_KINDS: readonly FilterKind[] = ["energy", "noEnergy"];

export function sameFilter(a: TableFilter, b: TableFilter): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "format") return a.format === (b as typeof a).format;
  if (a.kind === "energy") return a.level === (b as typeof a).level;
  return true;
}

/** `filters` with `filter` added: nothing changes if it is there already, and its opposite goes. */
export function addFilter(filters: readonly TableFilter[], filter: TableFilter): TableFilter[] {
  if (filters.some((f) => sameFilter(f, filter))) return [...filters];
  const opposite = OPPOSITES.flatMap(([a, b]) =>
    filter.kind === a ? [b] : filter.kind === b ? [a] : [],
  );
  if (ENERGY_KINDS.includes(filter.kind)) opposite.push(...ENERGY_KINDS);
  return [...filters.filter((f) => !opposite.includes(f.kind)), filter];
}

export function removeFilter(filters: readonly TableFilter[], filter: TableFilter): TableFilter[] {
  return filters.filter((f) => !sameFilter(f, filter));
}

/**
 * The formats the app names (`trackTable:format.*`): the names of the
 * backend's `SniffedFormat`. A test holds this list to the locale's keys.
 */
export const NAMED_FORMATS = [
  "mp3",
  "wav",
  "rf64",
  "aiff",
  "aifc",
  "flac",
  "mp4",
  "adts",
  "ogg_vorbis",
  "ogg_opus",
  "ogg",
  "asf",
  "wavpack",
  "musepack",
  "ape",
  "unknown",
] as const;

type NamedFormat = (typeof NAMED_FORMATS)[number];

function isNamedFormat(format: string): format is NamedFormat {
  return (NAMED_FORMATS as readonly string[]).includes(format);
}

/** A file format's name, e.g. `FLAC`; a format this build doesn't name shows as it is stored. */
export function formatName(format: string, t: TFunction<"trackTable">): string {
  return isNamedFormat(format) ? t(`format.${format}`) : format.toUpperCase();
}

/** A chip's text. */
export function filterLabel(filter: TableFilter, t: TFunction<"trackTable">): string {
  switch (filter.kind) {
    case "format":
      return t("filters.format", { format: formatName(filter.format, t) });
    case "missing":
      return t("filters.missing");
    case "inRekordbox":
      return t("filters.inRekordbox");
    case "notInRekordbox":
      return t("filters.notInRekordbox");
    case "qualityProblem":
      return t("filters.qualityProblem");
    case "inCrate":
      return t("filters.inCrate");
    case "inNoCrate":
      return t("filters.inNoCrate");
    case "inLibrary":
      return t("filters.inLibrary");
    case "notInLibrary":
      return t("filters.notInLibrary");
    case "lowQuality":
      return t("filters.lowQuality");
    case "analyzedByRekordbox":
      return t("filters.analyzedByRekordbox");
    case "notAnalyzedByRekordbox":
      return t("filters.notAnalyzedByRekordbox");
    case "energy":
      return t("filters.energy", { level: filter.level });
    case "noEnergy":
      return t("filters.noEnergy");
    case "missingTags":
      return t("filters.missingTags");
    case "missingArt":
      return t("filters.missingArt");
  }
}
