import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { ErrorKind, ReviewComparison, SwitchEffect, SwitchRefusal } from "../bindings";
import {
  useDismissBetterFile,
  useKeepAnyway,
  useSwitchLinkedFile,
  useWantBetterFile,
} from "./useReviewActions";
import styles from "./ReviewComparison.module.css";
import screenStyles from "./ReviewScreen.module.css";

/** The error kind that says why a switch is refused: its words are the error's. */
const REFUSAL_KIND: Record<SwitchRefusal, ErrorKind> = {
  linkedOnDisk: "reviewSwitchLinkedOnDisk",
  notInTrack: "reviewSwitchNotInTrack",
  targetGone: "reviewSwitchTargetGone",
  targetLinked: "reviewSwitchTargetLinked",
  entryOfAnotherTrack: "reviewSwitchEntryTaken",
};

/**
 * What a switch means in rekordbox, as a key of the review namespace. A track
 * rekordbox doesn't know has nothing to say: nothing of rekordbox's changes.
 */
const EFFECT_TEXT: Record<SwitchEffect, "switch.keepsEntry" | "switch.newTrack" | "switch.otherEntry" | null> = {
  notInRekordbox: null,
  keepsEntry: "switch.keepsEntry",
  newTrack: "switch.newTrack",
  otherEntry: "switch.otherEntry",
};

/**
 * The actions for the selected item (1bE-5, 1bE-7), under the comparison
 * table. Buttons only. Which ones depends on the item's kind:
 * - "Better file available": Dismiss.
 * - "Quality problem": Keep anyway, Find a better file.
 * - "Linked file gone, copy found": Switch file, with what the switch means
 *   in rekordbox said first, for a track rekordbox knows.
 *
 * The layout is the designer's (docs/design-style.md, "Review screen"): one
 * row of buttons directly under the table; the one filled button (`primary`)
 * is the one that takes the suggested file, the others are quiet. What a
 * switch means in rekordbox, and why a button is greyed out, are lines under
 * the row.
 *
 * Every action can be undone from the top bar's Undo. Give it a `key` per
 * item, so a refusal shown for one item doesn't stay under the next.
 */
export function ReviewActions({ comparison }: { comparison: ReviewComparison }) {
  const { t } = useTranslation("review");
  const dismiss = useDismissBetterFile();
  const keep = useKeepAnyway();
  const want = useWantBetterFile();
  const switchFile = useSwitchLinkedFile();
  const recordingId = comparison.id.a;
  const busy = dismiss.isPending || keep.isPending || want.isPending || switchFile.isPending;
  const failure = dismiss.error ?? keep.error ?? want.error ?? switchFile.error;

  let buttons: ReactNode = null;
  let note: string | null = null;
  let greyedOutBecause: string | null = null;
  if (comparison.kind === "betterFile" && comparison.suggestion) {
    const fileId = comparison.suggestion.what;
    buttons = (
      <button
        type="button"
        className={styles.button}
        disabled={busy}
        onClick={() => dismiss.mutate({ recordingId, fileId })}
      >
        {t("action.dismiss")}
      </button>
    );
  } else if (comparison.kind === "qualityProblem") {
    buttons = (
      <>
        <button
          type="button"
          className={styles.button}
          disabled={busy}
          onClick={() => keep.mutate(recordingId)}
        >
          {t("action.keepAnyway")}
        </button>
        <button
          type="button"
          className={styles.button}
          disabled={busy}
          onClick={() => want.mutate(recordingId)}
        >
          {t("action.findBetter")}
        </button>
      </>
    );
  } else if (comparison.kind === "clearBestFile" && comparison.switch) {
    const { fileId, effect, refusal } = comparison.switch;
    const effectText = effect === null ? null : EFFECT_TEXT[effect];
    note = effectText === null ? null : t(effectText);
    greyedOutBecause =
      refusal === null ? null : errorMessage({ kind: REFUSAL_KIND[refusal], params: {} });
    buttons = (
      <button
        type="button"
        className={styles.primary}
        disabled={busy || effect === null}
        onClick={() => {
          if (effect !== null) switchFile.mutate({ recordingId, fileId, effect });
        }}
      >
        {t("action.switch")}
      </button>
    );
  }
  if (buttons === null) return null;

  return (
    <>
      <div role="group" aria-label={t("action.label")} className={styles.actions}>
        {buttons}
      </div>
      {note !== null && (
        <p className={styles.doubt}>
          <span className={styles.doubtLabel}>{note}</span>
        </p>
      )}
      {greyedOutBecause !== null && <p className={styles.doubt}>{greyedOutBecause}</p>}
      {failure && (
        <p role="alert" className={screenStyles.error}>
          {errorMessage(failure)}
        </p>
      )}
    </>
  );
}
