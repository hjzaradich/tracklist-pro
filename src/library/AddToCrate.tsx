import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate, LibraryTrack } from "../bindings";
import { useUndoAfterAction } from "../shell/useUndo";
import { shownTitle } from "./RemoveFromLibrary";
import styles from "./LibraryTrackList.module.css";

/**
 * Adds a Library track to a crate that already exists (1aG-8). Choosing a
 * crate adds the track at once; the list goes back to its first line, ready
 * for the next track. With no crates there is nothing to choose, and it says so.
 */
export function AddToCrate({
  track,
  crates,
  onChoose,
}: {
  track: LibraryTrack;
  crates: Crate[];
  onChoose: (track: LibraryTrack, crate: Crate) => void;
}) {
  const { t } = useTranslation("crates");
  return (
    <select
      className={styles.select}
      aria-label={t("addTrack.labelFor", { title: shownTitle(track) })}
      disabled={crates.length === 0}
      value=""
      onChange={(event) => {
        const crate = crates.find((c) => String(c.id) === event.target.value);
        if (crate !== undefined) onChoose(track, crate);
      }}
    >
      <option value="">{crates.length === 0 ? t("addTrack.none") : t("addTrack.label")}</option>
      {crates.map((crate) => (
        <option key={crate.id} value={crate.id}>
          {crate.name}
        </option>
      ))}
    </select>
  );
}

/**
 * What the last "Add to crate" did, with Undo while it changed something. A
 * track that was in the crate already changes nothing, so there is nothing to undo.
 */
export function AddedToCrate({ crate, changed }: { crate: string; changed: boolean }) {
  const { t } = useTranslation("crates");
  const { arm, offered, undo } = useUndoAfterAction();
  // Shown anew for each add (the caller gives it a new key): the add that
  // just finished is the one its Undo is for.
  useEffect(() => {
    if (changed) arm();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once, when shown
  }, []);
  if (undo.data?.status === "undone") {
    return (
      <p role="status" className={styles.status}>
        {t("undone")}
      </p>
    );
  }
  return (
    <>
      <p role="status" className={styles.status}>
        {changed ? t("addTrack.added", { crate }) : t("addTrack.alreadyIn", { crate })}
        {changed && offered && (
          <button
            type="button"
            className={styles.button}
            disabled={undo.isPending}
            onClick={() => undo.mutate()}
          >
            {t("undo")}
          </button>
        )}
      </p>
      {undo.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(undo.error)}
        </p>
      )}
      {undo.data !== undefined && (
        <p role="alert" className={styles.error}>
          {t("undoRefused")}
        </p>
      )}
    </>
  );
}
