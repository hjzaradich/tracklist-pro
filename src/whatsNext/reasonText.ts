import type { TFunction } from "i18next";
import type { NextReason } from "../bindings";
import { formatKey, type KeyNotation } from "../keys/keyNotation";
import { signedBpm } from "../trackTable/fitText";

/** Whether a difference writes as none at all (`+0.0`). */
const isNone = (delta: number) => Math.abs(delta) < 0.05;

/** The signed step: `+1`, `-2`. */
const signedStep = (step: number) => (step > 0 ? `+${step}` : `${step}`);

/**
 * One reason in words, from its code and its facts: `next:reason.<code>`
 * (the Details panel never builds text in code). Keys are shown in the
 * user's notation.
 */
export function reasonText(reason: NextReason, t: TFunction<"next">, notation: KeyNotation): string {
  const key = (camelot: string | null) => (camelot === null ? "" : (formatKey(camelot, notation) ?? camelot));
  switch (reason.code) {
    case "keySame":
      return t("reason.keySame", { key: key(reason.from) });
    case "keyAdjacent":
    case "keyRelative":
    case "keyTwoSteps":
    case "keyDiagonal":
    case "keyFar":
      return t(`reason.${reason.code}`, { from: key(reason.from), to: key(reason.to) });
    case "bpm":
    case "bpmHalf":
    case "bpmDouble": {
      const delta = reason.delta ?? 0;
      if (reason.code === "bpm" && isNone(delta)) return t("reason.bpmSame");
      const bpm = t("reason.bpm", { delta: signedBpm(delta) });
      return reason.code === "bpm" ? bpm : t(`reason.${reason.code}`, { bpm });
    }
    case "energyStep":
      return reason.step === null || reason.step === 0
        ? t("reason.energySame")
        : t("reason.energyStep", { step: signedStep(reason.step) });
    case "playedAfter":
      return t("reason.playedAfter", { count: reason.count ?? 0 });
    case "keyNone":
    case "bpmNone":
    case "energyNone":
      return t(`reason.${reason.code}`);
  }
}
