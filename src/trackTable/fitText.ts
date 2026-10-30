import type { TFunction } from "i18next";
import type { BpmFit, FitReason, KeyFit, RowFit } from "../bindings";
import { formatKey, type KeyNotation } from "../keys/keyNotation";

/** The most dots the Fit column shows. */
export const MOST_DOTS = 3;

/** A BPM difference with its sign and one decimal: `+1.5`, `-2.0`. */
export function signedBpm(delta: number): string {
  const text = Math.abs(delta).toFixed(1);
  return delta < 0 ? `-${text}` : `+${text}`;
}

/** Whether the difference writes as no difference at all (`+0.0`). */
function isNone(delta: number): boolean {
  return Math.abs(delta) < 0.05;
}

function keyText(fit: KeyFit, t: TFunction<"trackTable">, notation: KeyNotation): string {
  const name = (camelot: string) => formatKey(camelot, notation) ?? camelot;
  switch (fit.kind) {
    case "same":
      return t("fit.sameKey", { key: name(fit.key) });
    case "neighbor":
      return t("fit.keyChange", { from: name(fit.from), to: name(fit.to) });
    case "far":
      return t("fit.keyFar", { from: name(fit.from), to: name(fit.to) });
    case "noKey":
      return t("fit.noKey");
    case "noTargetKey":
      return t("fit.noTargetKey");
  }
}

function bpmText(fit: BpmFit, t: TFunction<"trackTable">): string {
  switch (fit.kind) {
    case "delta": {
      // A difference that did not cross as a number (JSON has no NaN) is no BPM to compare.
      if (fit.delta === null) return t("fit.noBpm");
      const bpm = isNone(fit.delta) ? t("fit.sameBpm") : t("fit.bpm", { delta: signedBpm(fit.delta) });
      if (fit.time === "half") return t("fit.halfTime", { bpm });
      if (fit.time === "double") return t("fit.doubleTime", { bpm });
      return bpm;
    }
    case "noBpm":
      return t("fit.noBpm");
    case "noTargetBpm":
      return t("fit.noTargetBpm");
  }
}

/**
 * Why a track fits (or doesn't), in words: the key, then the tempo, joined
 * with ", " ("8A → 9A, +1.5 BPM"). Whatever is missing is said plainly. When
 * the selected track has neither a key nor a BPM, that is the one fact said.
 */
export function reasonText(reason: FitReason, t: TFunction<"trackTable">, notation: KeyNotation): string {
  if (reason.key.kind === "noTargetKey" && reason.bpm.kind === "noTargetBpm") return t("fit.noTarget");
  return [keyText(reason.key, t, notation), bpmText(reason.bpm, t)].join(t("fit.separator"));
}

/** The Fit column's text for a row: the reason, or that this is the selected track. */
export function fitText(fit: RowFit | null, t: TFunction<"trackTable">, notation: KeyNotation): string {
  if (fit === null) return "";
  return fit.reason === null ? t("fit.selected") : reasonText(fit.reason, t, notation);
}
