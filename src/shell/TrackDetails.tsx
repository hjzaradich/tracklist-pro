import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { FieldSources, SourcedValue, TableRow, TagField } from "../bindings";
import { useUndoneCount } from "./undoneStore";
import { useWithdrawRelink } from "../review/useReviewActions";
import { useTrackTagSources } from "./useTrackTagSources";
import styles from "./TrackDetails.module.css";

/** The label of each field, as a locale key. */
const FIELD_LABEL = {
  title: "details.field.title",
  artist: "details.field.artist",
  album: "details.field.album",
  genre: "details.field.genre",
  composer: "details.field.composer",
  remixer: "details.field.remixer",
  label: "details.field.label",
  year: "details.field.year",
  comment: "details.field.comment",
} as const satisfies Record<TagField, string>;

/** A value that is a file's tag names the file; rekordbox's is just rekordbox. */
function Source({ value }: { value: SourcedValue }) {
  const { t } = useTranslation("shell");
  return (
    <span className={styles.source}>
      {value.file === null
        ? t("details.source.rekordbox")
        : t("details.source.file", { file: value.file.name })}
    </span>
  );
}

/**
 * One field: its value and where it comes from. The other values the
 * track's sources hold are counted under the value, and listed, each naming
 * its file, once "More details" is on.
 */
function Field({ field, more }: { field: FieldSources; more: boolean }) {
  const { t } = useTranslation("shell");
  const { shown, others } = field;
  return (
    <div className={styles.field}>
      <dt className={styles.name}>{t(FIELD_LABEL[field.field])}</dt>
      <dd className={styles.value}>
        {shown === null ? (
          <span className={styles.source}>{t("details.source.blankInRekordbox")}</span>
        ) : (
          <>
            <span className={styles.text}>{shown.value}</span>
            <Source value={shown} />
          </>
        )}
        {others.length > 0 && !more && (
          <span className={styles.source}> {t("details.source.other", { count: others.length })}</span>
        )}
        {others.length > 0 && more && (
          <ul className={styles.others}>
            {others.map((other, index) => (
              <li key={index}>
                <span className={styles.text}>{other.value}</span>
                <Source value={other} />
              </li>
            ))}
          </ul>
        )}
      </dd>
    </div>
  );
}

/**
 * The Details panel's content for the selected track (1cB-17): its full
 * title and the full path of its file, wrapped and never cut off, then each
 * tag field with its value and where the value comes from. "More details"
 * (as in Review's comparison) lists the values that differ, each naming its
 * file. For a Library track whose rekordbox entry the user confirmed to its
 * file, a quiet text button withdraws the confirmation: one undoable step
 * (Undo is in the top bar), and the entry is decided again from scratch.
 *
 * Give it a `key` per track, so one track's status line doesn't stay under
 * the next.
 */
export function TrackDetails({ row }: { row: TableRow }) {
  const { t } = useTranslation("shell");
  const sources = useTrackTagSources(row.recordingId);
  const [more, setMore] = useState(false);
  const [withdrawn, setWithdrawn] = useState(false);
  const withdraw = useWithdrawRelink(() => setWithdrawn(true));
  const moreId = useId();
  // The line describes a withdrawal that no longer stands once it is undone.
  const undone = useUndoneCount((state) => state.count);
  const [seenUndone, setSeenUndone] = useState(undone);
  if (seenUndone !== undone) {
    setSeenUndone(undone);
    setWithdrawn(false);
  }

  const entry = sources.data?.confirmedEntry ?? null;
  return (
    <div className={styles.details}>
      {row.title !== null && <h2 className={styles.title}>{row.title}</h2>}
      {row.file !== null && (
        <div className={styles.file}>
          <span className={styles.name}>{t("details.file")}</span>
          <p className={styles.path}>{row.file.path}</p>
        </div>
      )}
      {sources.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(sources.error)}
        </p>
      )}
      {sources.isPending && <p className={styles.quiet}>{t("details.loadingTags")}</p>}
      {sources.data !== undefined && sources.data.fields.length > 0 && (
        <section aria-labelledby={`${moreId}-tags`}>
          <div className={styles.tagsTop}>
            <h3 id={`${moreId}-tags`} className={styles.heading}>
              {t("details.tags")}
            </h3>
            <label className={styles.more}>
              <input type="checkbox" checked={more} onChange={(event) => setMore(event.target.checked)} />
              {t("details.moreDetails")}
            </label>
          </div>
          <dl className={styles.fields}>
            {sources.data.fields.map((field) => (
              <Field key={field.field} field={field} more={more} />
            ))}
          </dl>
        </section>
      )}
      {entry !== null && (
        <div className={styles.confirmed}>
          <p className={styles.quiet}>{t("details.withdraw.confirmed")}</p>
          <button
            type="button"
            className={styles.withdraw}
            disabled={withdraw.isPending}
            onClick={() => {
              setWithdrawn(false);
              withdraw.mutate(entry);
            }}
          >
            {t("details.withdraw.action")}
          </button>
          <p className={styles.quiet}>{t("details.withdraw.hint")}</p>
        </div>
      )}
      {withdraw.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(withdraw.error)}
        </p>
      )}
      {withdrawn && !withdraw.isError && (
        <p role="status" className={styles.quiet}>
          {t("details.withdraw.done")}
        </p>
      )}
    </div>
  );
}
