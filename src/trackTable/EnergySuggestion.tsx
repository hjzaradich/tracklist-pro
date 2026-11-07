import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { unwrap } from "../api/errors";
import { commands, type EnergySuggestion as Suggestion } from "../bindings";
import styles from "./EnergySuggestion.module.css";

/** The suggestion for a track, asked again when the estimates are (they end with the Analyze run). */
export const ENERGY_SUGGESTION_QUERY_KEY = ["estimate", "energySuggestion"] as const;

/**
 * The app's suggested energy for a track, with its basis, or `null` for none
 * (ROADMAP 3.7). Only a suggestion: the track has no energy from it.
 */
export function useEnergySuggestion(recordingId: number) {
  return useQuery<Suggestion | null, Error>({
    queryKey: [...ENERGY_SUGGESTION_QUERY_KEY, recordingId],
    queryFn: () => unwrap(commands.energySuggestion(recordingId)),
  });
}

/**
 * "Suggested: 7", why, and a "Use 7" button (3C-E1c; ROADMAP 3.7). It sits
 * beside the energy buttons and is never shown as the track's energy: the
 * button hands the number to `onUse`, which sets it as the user would (an
 * ordinary, undoable edit). With no suggestion, or while it is being worked
 * out, it shows nothing.
 */
export function EnergySuggestion({ recordingId, onUse }: { recordingId: number; onUse: (energy: number) => void }) {
  const { t } = useTranslation("estimate");
  const suggestion = useEnergySuggestion(recordingId).data;
  if (suggestion === undefined || suggestion === null) return null;
  const { value, basis } = suggestion;
  const reasons = basis
    .map(({ code, percentile }) => t(`suggestion.reason.${code}`, { percent: Math.round(percentile) }))
    .join(", ");
  return (
    <div className={styles.suggestion} data-testid="energy-suggestion">
      <p className={styles.label}>{t("suggestion.label", { energy: value })}</p>
      {basis.length > 0 && <p className={styles.basis}>{t("suggestion.basis", { reasons })}</p>}
      <button type="button" className={styles.use} onClick={() => onUse(value)}>
        {t("suggestion.use", { energy: value })}
      </button>
    </div>
  );
}
