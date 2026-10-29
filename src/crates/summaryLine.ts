import type { TFunction } from "i18next";
import type { CrateSummary } from "../bindings";
import { formatKey, type KeyNotation } from "../keys/keyNotation";

/** The most keys the summary names before it says how many more there are. */
export const MOST_KEYS = 5;

/** A number with at most one decimal and no trailing zero: `124`, `124.5`. */
function plain(value: number): string {
  return String(Math.round(value * 10) / 10);
}

/**
 * The parts of a crate's summary line, in order: BPM range, energy range,
 * keys, top genres. A part with no data is left out. A crate with no tracks
 * has the one part that says so; tracks that have none of the four have the
 * one part that says that. The keys are written in the user's notation, in
 * Camelot order, at most {@link MOST_KEYS} of them and then "+N".
 */
export function summaryParts(
  summary: CrateSummary,
  notation: KeyNotation,
  t: TFunction<"crates">,
): string[] {
  if (summary.tracks === 0) return [t("summary.empty")];
  const parts: string[] = [];
  if (summary.bpm !== null) {
    const { min, max } = summary.bpm;
    parts.push(
      plain(min) === plain(max)
        ? t("summary.bpmOne", { bpm: plain(min) })
        : t("summary.bpm", { min: plain(min), max: plain(max) }),
    );
  }
  if (summary.energy !== null) {
    const { min, max } = summary.energy;
    parts.push(
      min === max ? t("summary.energyOne", { energy: min }) : t("summary.energy", { min, max }),
    );
  }
  if (summary.keys.length > 0) {
    const names = summary.keys.map((key) => formatKey(key, notation) ?? key);
    const listed = names.slice(0, MOST_KEYS).join(", ");
    const more = names.length - MOST_KEYS;
    parts.push(
      more > 0
        ? t("summary.keysMore", { keys: listed, more })
        : t("summary.keys", { keys: listed, count: names.length }),
    );
  }
  if (summary.genres.length > 0) parts.push(summary.genres.join(", "));
  return parts.length > 0 ? parts : [t("summary.nothing")];
}
