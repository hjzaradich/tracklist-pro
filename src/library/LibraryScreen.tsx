import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate } from "../bindings";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { useWhenUndone } from "../shell/undoneStore";
import { TrackTableView } from "../trackTable/TrackTableView";
import styles from "./LibraryTrackList.module.css";
import { useAddToCrate, useCrates } from "../crates/useCrates";
import { AddedToCrate, AddToCrate } from "./AddToCrate";
import { ConfirmRemove, RemoveButton } from "./RemoveFromLibrary";
import { libraryRow, type LibraryRow } from "./libraryRow";
import { LIBRARY_TABLE_QUERY_KEY } from "./useLibraryTracks";
import { useRemoveLibraryTrack } from "./useRemoveLibraryTrack";

const LIBRARY_SOURCE = { kind: "library" } as const;

/**
 * The Library: the tracks the user added to play with (1aD-6), in the track
 * table (1cA-3). Starting a Library and adding tracks arrive in 1aE. A track
 * is removed after a confirmation, and the removal can be undone (1aE-6) from
 * the top bar or with Ctrl+Z; this screen offers no Undo of its own (owner
 * decision, 2026-10-27).
 */
export function LibraryScreen() {
  const { t } = useTranslation("library");
  const remove = useRemoveLibraryTrack();
  const [pending, setPending] = useState<LibraryRow | null>(null);
  const [removed, setRemoved] = useState(false);
  const crates = useCrates();
  const addToCrate = useAddToCrate();
  // What the last "Add to crate" did; a new `key` announces the line anew.
  const [added, setAdded] = useState<{
    key: number;
    crate: string;
    changed: boolean;
  } | null>(null);

  // Undone from the top bar: what the lines say no longer stands.
  useWhenUndone(() => {
    setRemoved(false);
    setAdded(null);
  });

  const add = (track: LibraryRow, crate: Crate) => {
    setRemoved(false);
    remove.reset();
    addToCrate.mutate(
      { id: crate.id, tracks: [track.id] },
      {
        onSuccess: (result) =>
          setAdded((previous) => ({
            key: (previous?.key ?? 0) + 1,
            crate: crate.name,
            changed: result.operationId !== null,
          })),
      },
    );
  };

  const confirm = () => {
    if (pending === null) return;
    addToCrate.reset();
    remove.mutate(pending.id, {
      onSuccess: () => setRemoved(true),
    });
    setPending(null);
    setRemoved(false);
    setAdded(null);
  };

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
        {removed && (
          <p role="status" className={styles.status}>
            {t("remove.done")}
          </p>
        )}
        {addToCrate.isError && (
          <p role="alert" className={styles.error}>
            {errorMessage(addToCrate.error)}
          </p>
        )}
        {added !== null && !addToCrate.isError && (
          <AddedToCrate key={added.key} crate={added.crate} changed={added.changed} />
        )}
        <TrackTableView
          screen="library"
          source={LIBRARY_SOURCE}
          keyPrefix={LIBRARY_TABLE_QUERY_KEY}
          // How many tracks the Library holds: the list itself is too long
          // to count.
          count={(total) => t("trackCount", { count: total })}
          empty={
            <div className={styles.centered}>
              <EmptyState>{t("empty")}</EmptyState>
            </div>
          }
          actionsWidth={232}
          actions={(row) => {
            const track = libraryRow(row);
            return (
              track && (
                <>
                  <AddToCrate track={track} crates={crates.data ?? []} onChoose={add} />
                  <RemoveButton
                    track={track}
                    onRemove={(chosen) => {
                      setRemoved(false);
                      setAdded(null);
                      setPending(chosen);
                    }}
                  />
                </>
              )
            );
          }}
        />
      </div>
    </StageScreen>
  );
}
