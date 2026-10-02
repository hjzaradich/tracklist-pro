import { useEffect, useId, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { LibraryTrack } from "../bindings";
import styles from "./LibraryTrackList.module.css";

/** What the Library list calls a track: its title, or its file's name. */
export function shownTitle(track: LibraryTrack): string {
  return track.title ?? track.file?.name ?? "";
}

/**
 * Asks before a track leaves the Library (1aE-6). Nothing happens until the
 * user confirms, and the file on disk is never touched.
 */
export function ConfirmRemove({
  track,
  onConfirm,
  onCancel,
}: {
  track: LibraryTrack;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("library");
  // The dialog takes focus, on the safe choice.
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  const titleId = useId();
  const detailId = useId();
  const conflictsId = useId();
  const hasConflicts = track.openConflicts > 0;
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={hasConflicts ? `${detailId} ${conflictsId}` : detailId}
      className={styles.confirm}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <p id={titleId} className={styles.confirmTitle}>
        {t("remove.title", { title: shownTitle(track) })}
      </p>
      <p id={detailId} className={styles.confirmDetail}>
        {t("remove.detail")}
      </p>
      {hasConflicts && (
        <p id={conflictsId} className={styles.confirmDetail}>
          {t("remove.conflicts", { count: track.openConflicts })}
        </p>
      )}
      <div className={styles.actions}>
        <button type="button" className={styles.primary} onClick={onConfirm}>
          {t("remove.confirm")}
        </button>
        <button ref={cancel} type="button" className={styles.button} onClick={onCancel}>
          {t("remove.cancel")}
        </button>
      </div>
    </div>
  );
}

/** The Remove button of a Library row. Asking comes first ({@link ConfirmRemove}). */
export function RemoveButton({
  track,
  onRemove,
}: {
  track: LibraryTrack;
  onRemove: (track: LibraryTrack) => void;
}) {
  const { t } = useTranslation("library");
  return (
    <button
      type="button"
      className={styles.button}
      aria-label={t("remove.buttonFor", { title: shownTitle(track) })}
      onClick={() => onRemove(track)}
    >
      {t("remove.button")}
    </button>
  );
}
