import type { TFunction } from "i18next";
import type { BridgeReason, TimelineEntry } from "../bindings";
import { formatKey, type KeyNotation } from "../keys/keyNotation";

/** A number to one place after the point, whole when it is whole. */
const bpm = (n: number) => String(Math.round(n * 10) / 10);

/**
 * Which neighbours a reason has, as the end of its key: both (no ending), only
 * the one before (`After`: the gap comes after it), only the one after
 * (`Before`), or neither (`Only`).
 */
function ending(from: unknown, to: unknown): "" | "After" | "Before" | "Only" {
  if (from !== null && to !== null) return "";
  if (from !== null) return "After";
  return to !== null ? "Before" : "Only";
}

/**
 * Why a track fills a gap, in words (`playlists:gap.<code>`, ROADMAP 3.5; the
 * backend sends the facts, never the sentence). Keys come in the user's
 * notation. A reason is a short fragment; the screen joins them with ", ".
 * A reason that is missing a number it needs (a tempo that isn't there) has
 * no words and gives `null`: it is left out, never worded with a made-up 0.
 */
export function reasonText(t: TFunction<"playlists">, reason: BridgeReason, notation: KeyNotation): string | null {
  const key = (camelot: string | null) => formatKey(camelot, notation) ?? camelot ?? "";
  switch (reason.code) {
    case "keyBridge":
      return t("gap.keyBridge", { from: key(reason.from), via: key(reason.via), to: key(reason.to) });
    case "keyFits":
      return t(`gap.keyFits${ending(reason.from, reason.to)}`, {
        from: key(reason.from),
        via: key(reason.via),
        to: key(reason.to),
      });
    case "bpmStep":
      if (reason.from === null || reason.via === null || reason.to === null) return null;
      return t("gap.bpmStep", { from: bpm(reason.from), via: bpm(reason.via), to: bpm(reason.to) });
    case "bpmFits":
      if (reason.via === null) return null;
      return t(`gap.bpmFits${ending(reason.from, reason.to)}`, {
        from: reason.from === null ? "" : bpm(reason.from),
        via: bpm(reason.via),
        to: reason.to === null ? "" : bpm(reason.to),
      });
    case "energyStep":
      return t(`gap.energyStep${ending(reason.from, reason.to)}`, { from: reason.from, via: reason.via, to: reason.to });
    case "energyTarget":
      return reason.target === null
        ? t("gap.energyStepOnly", { via: reason.via })
        : t("gap.energyTarget", { via: reason.via, target: bpm(reason.target) });
  }
}

/** Artist - Title, the way a track is written (docs/copy-style.md); empty when it has neither. */
export function entryName(entry: Pick<TimelineEntry, "artist" | "title">): string {
  return [entry.artist, entry.title].filter((part) => part !== null && part !== "").join(" - ");
}
