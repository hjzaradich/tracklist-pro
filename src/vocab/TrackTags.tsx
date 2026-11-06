import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { TableRow } from "../bindings";
import { colourOf } from "./colours";
import styles from "./vocab.module.css";
import { useApplyTag, useRemoveTag, useTagsOfTrack, useVocab } from "./useVocab";

/**
 * The tags in the Details panel (3A-T): every tag as a chip that is pressed
 * while the track has it. A press toggles it, one operation (Undo, in the top
 * bar, takes it back). The vocabulary is made on the Tags screen, so with no
 * tags this says so and shows nothing else.
 *
 * Give it a `key` per track, so one track's error doesn't stay under the next.
 */
export function TrackTags({ row }: { row: TableRow }) {
  const { t } = useTranslation("vocab");
  const vocab = useVocab();
  const have = useTagsOfTrack(row.recordingId);
  const apply = useApplyTag();
  const remove = useRemoveTag();
  const headingId = useId();
  if (vocab.data === undefined || have.data === undefined) {
    return vocab.isError ? (
      <p role="alert" className={styles.error}>
        {errorMessage(vocab.error)}
      </p>
    ) : null;
  }
  const groups = vocab.data.filter((group) => group.tags.length > 0);
  const failure = apply.error ?? remove.error;
  return (
    <section aria-labelledby={headingId}>
      <h3 id={headingId} className={styles.heading}>
        {t("track.title")}
      </h3>
      {groups.length === 0 && <p className={styles.quiet}>{t("track.none")}</p>}
      {groups.map((group) => (
        <div key={group.id} className={styles.chipGroup} role="group" aria-label={group.name}>
          <span className={styles.groupName}>{group.name}</span>
          {group.tags.map((tag) => {
            const on = have.data.includes(tag.id);
            return (
              <button
                key={tag.id}
                type="button"
                className={styles.chip}
                aria-pressed={on}
                disabled={apply.isPending || remove.isPending}
                onClick={() => (on ? remove : apply).mutate({ tag: tag.id, recordings: [row.recordingId] })}
              >
                <span className={styles.swatch} style={{ background: colourOf(tag, group) }} aria-hidden="true" />
                {tag.name}
              </button>
            );
          })}
        </div>
      ))}
      {failure !== null && (
        <p role="alert" className={styles.error}>
          {errorMessage(failure)}
        </p>
      )}
    </section>
  );
}
