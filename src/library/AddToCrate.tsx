import { useTranslation } from "react-i18next";
import type { Crate } from "../bindings";
import { crateChoices } from "../crates/tree";
import type { LibraryRow } from "./libraryRow";
import { shownTitle } from "./RemoveFromLibrary";
import styles from "./LibraryTrackList.module.css";

/**
 * Adds a Library track to a crate that already exists (1aG-8). Choosing a
 * crate adds the track at once; the list goes back to its first line, ready
 * for the next track. Crates in folders are named by where they are
 * ("Friday residency › Warmup"); folders themselves can't be chosen, as they
 * hold no tracks. With no crates there is nothing to choose, and it says so.
 * `crates` is the whole list from the backend, folders included.
 */
export function AddToCrate({
  track,
  crates,
  onChoose,
}: {
  track: LibraryRow;
  crates: Crate[];
  onChoose: (track: LibraryRow, crate: Crate) => void;
}) {
  const { t } = useTranslation("crates");
  const choices = crateChoices(crates);
  return (
    <select
      className={styles.select}
      aria-label={t("addTrack.labelFor", { title: shownTitle(track) })}
      disabled={choices.length === 0}
      value=""
      onChange={(event) => {
        const chosen = choices.find((c) => String(c.crate.id) === event.target.value);
        if (chosen !== undefined) onChoose(track, chosen.crate);
      }}
    >
      <option value="">{choices.length === 0 ? t("addTrack.none") : t("addTrack.label")}</option>
      {choices.map(({ crate, label }) => (
        <option key={crate.id} value={crate.id}>
          {label}
        </option>
      ))}
    </select>
  );
}

/**
 * What the last "Add to crate" did. `changed` is false for a track that was
 * in the crate already. No Undo here: Undo is in the top bar, and Ctrl+Z
 * (owner decision, 2026-10-27). The caller gives it a new key for each add,
 * so the line is announced anew.
 */
export function AddedToCrate({ crate, changed }: { crate: string; changed: boolean }) {
  const { t } = useTranslation("crates");
  return (
    <p role="status" className={styles.status}>
      {changed ? t("addTrack.added", { crate }) : t("addTrack.alreadyIn", { crate })}
    </p>
  );
}
