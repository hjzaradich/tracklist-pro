import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { errorMessage } from "../api/errors";
import type { MergeChoice, ReviewComparison, SameRecordingOffer, SameRecordingTrack } from "../bindings";
import { MergeFields, pickedFor } from "./MergeFields";
import { trackLabel } from "./ReviewQueue";
import { useKeepBoth, useMergeQuestions, useMergeTracks } from "./useReviewActions";
import { useReviewKeys, type ReviewKey } from "./useReviewKeys";
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
 * The confirmation also asks field by field (1cB-20): where the tracks hold
 * different values and one is the user's, a row per field with each track's
 * value, the staying track's picked at first (`MergeFields`). What is picked
 * goes with "Merge", and the one Undo takes the merge and the values back.
 *
 * Tracks linked as versions are never merged: then only "Keep both" is
 * offered, the filled button, with the line that says why.
 *
 * The item's Library tracks are sent back with each action, so one whose
 * tracks changed under the screen is refused, not answered unseen.
 *
 * The keys (owner decision, 2026-10-16): Enter does nothing here (a merge
 * needs its confirmation), and a number key is a row of the tracks table: it
 * opens the confirmation with that track chosen to stay. Nothing is merged
 * until "Merge" there. No key while the tracks are linked versions, and none
 * while the confirmation is open (it is a dialog: the screen's keys wait).
 * `onDecided` is called when an action worked, as for the other kinds.
 */
export function SameRecordingActions({
  comparison,
  offer,
  onDecided,
}: {
  comparison: ReviewComparison;
  offer: SameRecordingOffer;
  onDecided?: () => void;
}) {
  const { t } = useTranslation("review");
  const keepBoth = useKeepBoth(onDecided);
  const merge = useMergeTracks(onDecided);
  const [choosing, setChoosing] = useState(false);
  const [keep, setKeep] = useState<number | null>(offer.keep ?? offer.tracks[0]?.recordingId ?? null);
  // The values picked, for the track that stays now: another track staying is
  // another set of rows, so the picks start over.
  const [picked, setPicked] = useState<{ keep: number | null; picks: Record<string, number> }>({
    keep: null,
    picks: {},
  });
  const picks = picked.keep === keep ? picked.picks : {};
  const goBack = useRef<HTMLButtonElement>(null);
  const chooseId = useId();
  const detailId = useId();
  useEffect(() => {
    if (choosing) goBack.current?.focus();
  }, [choosing]);

  const anchor = comparison.id.a;
  const recordingIds = offer.tracks.map((track) => track.recordingId);
  const questions = useMergeQuestions(anchor, recordingIds, keep, choosing && !offer.linkedVersions);
  const fields = questions.data ?? [];
  const names = new Map(offer.tracks.map((track) => [track.recordingId, nameOf(t, comparison, track)]));
  const busy = keepBoth.isPending || merge.isPending;
  const failure = keepBoth.error ?? merge.error ?? questions.error;
  // What the merge sends: the value picked for each row, the default included,
  // by the text the screen showed (the merge is refused if it changed).
  const choices: MergeChoice[] = fields.flatMap((field) => {
    const from = pickedFor(field, picks);
    const value = field.values.find((v) => v.recordingId === from)?.value;
    return value === null || value === undefined ? [] : [{ field: field.field, from, value }];
  });
  const leaving = Math.max(offer.tracks.length - 1, 1);

  // A number key chooses the track of that row and opens the confirmation.
  useReviewKeys((key: ReviewKey) => {
    // Enter and S do nothing on this item (owner decision, 2026-10-16).
    if (key === "enter" || key === "s" || offer.linkedVersions || choosing || busy) return false;
    const track = offer.tracks[key - 1];
    if (track === undefined) return false;
    setKeep(track.recordingId);
    setChoosing(true);
    return true;
  });

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
          <p className={styles.keys}>
            <span>{t("keys.tracks")}</span>
          </p>
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
          <MergeFields
            fields={fields}
            names={names}
            picks={picks}
            disabled={busy}
            onPick={(field, recordingId) =>
              setPicked({ keep, picks: { ...picks, [field]: recordingId } })
            }
          />
          <p id={detailId} className={styles.doubt}>
            {t("same.confirmValues", { count: leaving })}
          </p>
          <div role="group" aria-label={t("action.label")} className={styles.actions}>
            <button
              type="button"
              className={styles.primary}
              disabled={busy || keep === null || !questions.isSuccess}
              onClick={() => {
                if (keep !== null) merge.mutate({ anchor, recordingIds, keep, choices });
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
