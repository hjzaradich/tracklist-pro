import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { LibraryTrack } from "../bindings";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { LibraryTrackList } from "./LibraryTrackList";
import styles from "./LibraryTrackList.module.css";
import { ConfirmRemove } from "./RemoveFromLibrary";
import { useLibraryTracks } from "./useLibraryTracks";
import { useRemoveLibraryTrack, useUndoLast } from "./useRemoveLibraryTrack";

/** What the strip above the list says about the last removal. */
type Removal = "confirming" | "removed" | "undone" | "undoRefused";

/**
 * The Library: the tracks the user added to play with (1aD-6). Starting a
 * Library and adding tracks arrive in 1aE. A track is removed after a
 * confirmation, and the removal can be undone (1aE-6).
 */
export function LibraryScreen() {
  const { t } = useTranslation("library");
  const tracks = useLibraryTracks();
  const remove = useRemoveLibraryTrack();
  const undo = useUndoLast();
  const [pending, setPending] = useState<LibraryTrack | null>(null);
  const [removal, setRemoval] = useState<Removal | null>(null);

  const confirm = () => {
    if (pending === null) return;
    remove.mutate(pending.id, { onSuccess: () => setRemoval("removed") });
    setPending(null);
    setRemoval(null);
  };
  const undoRemoval = () =>
    undo.mutate(undefined, {
      onSuccess: (outcome) => setRemoval(outcome.status === "undone" ? "undone" : "undoRefused"),
    });

  return (
    <StageScreen stage="library">
      <div className={styles.content}>
        {pending !== null && (
          <ConfirmRemove
            track={pending}
            busy={remove.isPending}
            onConfirm={confirm}
            onCancel={() => setPending(null)}
          />
        )}
        {remove.isError && (
          <p role="alert" className={styles.error}>
            {errorMessage(remove.error)}
          </p>
        )}
        {removal === "removed" && (
          <p role="status" className={styles.status}>
            {t("remove.done")}
            <button
              type="button"
              className={styles.button}
              disabled={undo.isPending}
              onClick={undoRemoval}
            >
              {t("remove.undo")}
            </button>
          </p>
        )}
        {removal === "undone" && (
          <p role="status" className={styles.status}>
            {t("remove.undone")}
          </p>
        )}
        {(removal === "undoRefused" || undo.isError) && (
          <p role="alert" className={styles.error}>
            {undo.isError ? errorMessage(undo.error) : t("remove.undoRefused")}
          </p>
        )}
        {tracks.isError ? (
          <p role="alert" className={styles.error}>
            {errorMessage(tracks.error)}
          </p>
        ) : tracks.data === undefined ? null : tracks.data.length === 0 ? (
          <div className={styles.centered}>
            <EmptyState>{t("empty")}</EmptyState>
          </div>
        ) : (
          <div className={styles.scroll}>
            <LibraryTrackList
              tracks={tracks.data}
              onRemove={(track) => {
                setRemoval(null);
                setPending(track);
              }}
            />
          </div>
        )}
      </div>
    </StageScreen>
  );
}
