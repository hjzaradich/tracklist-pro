import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate, LibraryTrack } from "../bindings";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { LibraryTrackList } from "./LibraryTrackList";
import styles from "./LibraryTrackList.module.css";
import { useAddToCrate, useCrates } from "../crates/useCrates";
import { AddedToCrate, AddToCrate } from "./AddToCrate";
import { ConfirmRemove, RemoveButton } from "./RemoveFromLibrary";
import { useUndoAfterAction } from "../shell/useUndo";
import { useLibraryTracks } from "./useLibraryTracks";
import { useRemoveLibraryTrack } from "./useRemoveLibraryTrack";

/** What the strip above the list says about the last removal. */
type Removal = "removed" | "undone" | "nothingToUndo" | "undoRefused";

/**
 * The Library: the tracks the user added to play with (1aD-6). Starting a
 * Library and adding tracks arrive in 1aE. A track is removed after a
 * confirmation, and the removal can be undone (1aE-6).
 */
export function LibraryScreen() {
  const { t } = useTranslation("library");
  const tracks = useLibraryTracks();
  const remove = useRemoveLibraryTrack();
  const { arm, offered, undo } = useUndoAfterAction();
  const [pending, setPending] = useState<LibraryTrack | null>(null);
  const [removal, setRemoval] = useState<Removal | null>(null);
  const crates = useCrates();
  const addToCrate = useAddToCrate();
  // What the last "Add to crate" did; `key` starts its Undo afresh.
  const [added, setAdded] = useState<{
    key: number;
    crate: string;
    operationId: number | null;
  } | null>(null);

  const add = (track: LibraryTrack, crate: Crate) => {
    setRemoval(null);
    undo.reset();
    remove.reset();
    addToCrate.mutate(
      { id: crate.id, tracks: [track.id] },
      {
        onSuccess: (result) =>
          setAdded((previous) => ({
            key: (previous?.key ?? 0) + 1,
            crate: crate.name,
            operationId: result.operationId,
          })),
      },
    );
  };

  const confirm = () => {
    if (pending === null) return;
    undo.reset();
    addToCrate.reset();
    remove.mutate(pending.id, {
      onSuccess: () => {
        arm();
        setRemoval("removed");
      },
    });
    setPending(null);
    setRemoval(null);
    setAdded(null);
  };
  const undoRemoval = () =>
    undo.mutate(undefined, {
      onSuccess: (outcome) =>
        setRemoval(
          outcome.status === "undone"
            ? "undone"
            : outcome.status === "nothingToUndo"
              ? "nothingToUndo"
              : "undoRefused",
        ),
    });

  return (
    <StageScreen stage="library">
      <div className={styles.content}>
        {pending !== null && (
          <ConfirmRemove track={pending} onConfirm={confirm} onCancel={() => setPending(null)} />
        )}
        {remove.isError && (
          <p role="alert" className={styles.error}>
            {errorMessage(remove.error)}
          </p>
        )}
        {(removal === "removed" || removal === "nothingToUndo") && (
          <p role="status" className={styles.status}>
            {t("remove.done")}
            {/* Gone once the removal isn't the next step to undo any more. */}
            {offered && (
              <button
                type="button"
                className={styles.button}
                disabled={undo.isPending || removal === "nothingToUndo"}
                onClick={undoRemoval}
              >
                {t("remove.undo")}
              </button>
            )}
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
        {addToCrate.isError && (
          <p role="alert" className={styles.error}>
            {errorMessage(addToCrate.error)}
          </p>
        )}
        {added !== null && !addToCrate.isError && (
          <AddedToCrate key={added.key} crate={added.crate} operationId={added.operationId} />
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
              actions={(track) => (
                <>
                  <AddToCrate track={track} crates={crates.data ?? []} onChoose={add} />
                  <RemoveButton
                    track={track}
                    onRemove={(chosen) => {
                      setRemoval(null);
                      setAdded(null);
                      undo.reset();
                      setPending(chosen);
                    }}
                  />
                </>
              )}
            />
          </div>
        )}
      </div>
    </StageScreen>
  );
}
