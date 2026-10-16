import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { errorMessage } from "../api/errors";
import type { ReviewComparison, SameRecordingOffer, SameRecordingTrack } from "../bindings";
import { trackLabel } from "./ReviewQueue";
import { useKeepBoth, useMergeTracks } from "./useReviewActions";
import styles from "./ReviewComparison.module.css";
import screenStyles from "./ReviewScreen.module.css";

/** A track's rating in rekordbox, as words. */
function stars(t: TFunction<"review">, count: number): string {
  return count === 0 ? t("same.noRating") : t("same.stars", { count });
}

/** The name of one of the item's Library tracks, as the queue names it. */
function nameOf(t: TFunction<"review">, comparison: ReviewComparison, track: SameRecordingTrack) {
  const named = comparison.tracks.find((tr) => tr.recordingId === track.recordingId);
  return named ? trackLabel(t, named) : t("item.untitled");
}

/** Each Library track of the item: what rekordbox holds for it, and its crates. */
function TrackFacts({ comparison, offer }: { comparison: ReviewComparison; offer: SameRecordingOffer }) {
  const { t } = useTranslation("review");
  return (
    <table className={styles.table} aria-label={t("same.label")}>
      <thead>
        <tr>
          <th scope="col">{t("col.track")}</th>
          <th scope="col">{t("same.col.cues")}</th>
          <th scope="col">{t("same.col.rating")}</th>
          <th scope="col">{t("same.col.plays")}</th>
          <th scope="col">{t("same.col.crates")}</th>
        </tr>
      </thead>
      <tbody>
        {offer.tracks.map((track) => (
          <tr key={track.recordingId} className={styles.row}>
            <th scope="row" className={styles.file}>
              <span className={styles.name}>{nameOf(t, comparison, track)}</span>
              {track.recordingId === offer.keep && (
                <span className={styles.marks}>
                  <span className={`${styles.mark} ${styles.markSuggested}`}>{t("mark.suggested")}</span>
                </span>
              )}
            </th>
            {track.rekordbox === null ? (
              <td colSpan={3}>{t("same.notInRekordbox")}</td>
            ) : (
              <>
                <td className={styles.number}>{track.rekordbox.cues}</td>
                <td>{stars(t, track.rekordbox.stars)}</td>
                <td className={styles.number}>{track.rekordbox.plays}</td>
              </>
            )}
            <td className={styles.number}>{track.crates}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * The actions of a "Same track twice in your Library" item (1bE-9): "Keep
 * both", or "Merge", the one filled button, which only opens the choice of
 * the track that stays (the suggested one chosen) with what a merge does; the
 * merge happens on that confirmation's "Merge", and "Go back" closes it. Both
 * can be undone from the top bar's Undo.
 *
 * Tracks linked as versions are never merged: then only "Keep both" is
 * offered, the filled button, with the line that says why.
 *
 * The item's Library tracks are sent back with each action, so one whose
 * tracks changed under the screen is refused, not answered unseen. No keys
 * yet: Enter and S do nothing on this item, and the number keys that will
 * pick the track that stays come with the Review keyboard handling.
 */
export function SameRecordingActions({
  comparison,
  offer,
}: {
  comparison: ReviewComparison;
  offer: SameRecordingOffer;
}) {
  const { t } = useTranslation("review");
  const keepBoth = useKeepBoth();
  const merge = useMergeTracks();
  const [choosing, setChoosing] = useState(false);
  const [keep, setKeep] = useState<number | null>(offer.keep ?? offer.tracks[0]?.recordingId ?? null);
  const goBack = useRef<HTMLButtonElement>(null);
  const chooseId = useId();
  const detailId = useId();
  useEffect(() => {
    if (choosing) goBack.current?.focus();
  }, [choosing]);

  const anchor = comparison.id.a;
  const recordingIds = offer.tracks.map((track) => track.recordingId);
  const busy = keepBoth.isPending || merge.isPending;
  const failure = keepBoth.error ?? merge.error;
  const leaving = Math.max(offer.tracks.length - 1, 1);

  return (
    <>
      <TrackFacts comparison={comparison} offer={offer} />
      {offer.linkedVersions ? (
        <>
          <div role="group" aria-label={t("action.label")} className={styles.actions}>
            <button
              type="button"
              className={styles.primary}
              disabled={busy}
              onClick={() => keepBoth.mutate({ anchor, recordingIds })}
            >
              {t("action.keepBoth")}
            </button>
          </div>
          <p className={styles.doubt}>
            {errorMessage({ kind: "reviewMergeLinkedVersions", params: {} })}
          </p>
        </>
      ) : !choosing ? (
        <div role="group" aria-label={t("action.label")} className={styles.actions}>
          <button
            type="button"
            className={styles.primary}
            disabled={busy}
            onClick={() => setChoosing(true)}
          >
            {t("action.merge")}
          </button>
          <button
            type="button"
            className={styles.button}
            disabled={busy}
            onClick={() => keepBoth.mutate({ anchor, recordingIds })}
          >
            {t("action.keepBoth")}
          </button>
        </div>
      ) : (
        <div
          role="alertdialog"
          aria-labelledby={chooseId}
          aria-describedby={detailId}
          onKeyDown={(event) => {
            if (event.key === "Escape") setChoosing(false);
          }}
        >
          <fieldset className={styles.choice}>
            <legend id={chooseId} className={styles.doubtLabel}>
              {t("same.choose")}
            </legend>
            {offer.tracks.map((track) => {
              const name = nameOf(t, comparison, track);
              return (
                <label key={track.recordingId} className={styles.choiceOption}>
                  <input
                    type="radio"
                    name={`${chooseId}-keep`}
                    checked={keep === track.recordingId}
                    onChange={() => setKeep(track.recordingId)}
                  />
                  {track.recordingId === offer.keep ? t("same.suggestedTrack", { name }) : name}
                </label>
              );
            })}
          </fieldset>
          <p id={detailId} className={styles.doubt}>
            {t("same.confirm", { count: leaving })}
          </p>
          <div role="group" aria-label={t("action.label")} className={styles.actions}>
            <button
              type="button"
              className={styles.primary}
              disabled={busy || keep === null}
              onClick={() => {
                if (keep !== null) merge.mutate({ anchor, recordingIds, keep });
              }}
            >
              {t("action.mergeConfirm")}
            </button>
            <button
              ref={goBack}
              type="button"
              className={styles.button}
              disabled={busy}
              onClick={() => setChoosing(false)}
            >
              {t("action.goBack")}
            </button>
          </div>
        </div>
      )}
      {failure && (
        <p role="alert" className={screenStyles.error}>
          {errorMessage(failure)}
        </p>
      )}
    </>
  );
}
