import { useEffect, useRef } from "react";
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
  busy,
  onConfirm,
  onCancel,
}: {
  track: LibraryTrack;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("library");
  // The dialog takes focus, on the safe choice.
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  return (
    <div
      role="alertdialog"
      aria-labelledby="remove-title"
      aria-describedby="remove-detail"
      className={styles.confirm}
    >
      <p id="remove-title" className={styles.confirmTitle}>
        {t("remove.title", { title: shownTitle(track) })}
      </p>
      <p id="remove-detail" className={styles.confirmDetail}>
        {t("remove.detail")}
      </p>
      {track.openConflicts > 0 && (
        <p className={styles.confirmDetail}>
          {t("remove.conflicts", { count: track.openConflicts })}
        </p>
      )}
      <div className={styles.actions}>
        <button type="button" className={styles.primary} disabled={busy} onClick={onConfirm}>
          {t("remove.confirm")}
        </button>
        <button
          ref={cancel}
          type="button"
          className={styles.button}
          disabled={busy}
          onClick={onCancel}
        >
          {t("remove.cancel")}
        </button>
      </div>
    </div>
  );
}
