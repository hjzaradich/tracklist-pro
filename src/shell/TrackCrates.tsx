import { Link } from "@tanstack/react-router";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { TableRow } from "../bindings";
import { PATH_SEPARATOR, pathOf, walk } from "../crates/tree";
import { useCrates, useCratesOfTrack, useRemoveFromCrate } from "../crates/useCrates";
import { useUndoneCount } from "./undoneStore";
import styles from "./TrackDetails.module.css";

/**
 * The crates the track is in (1cB-14; ROADMAP 1.14, "Membership"), in the
 * Details panel: each one a link to the crate, named by its place
 * ("Friday residency › Warmup"), with "Remove from this crate" as a plain
 * text action. Removing is one operation, so Undo, in the top bar, puts the
 * track back. Only a Library track can be in a crate, so any other track
 * shows nothing here.
 *
 * Give it a `key` per track, so one track's status line doesn't stay under
 * the next.
 */
export function TrackCrates({ row }: { row: TableRow }) {
  const { t } = useTranslation("shell");
  const libraryTrackId = row.libraryTrackId;
  const crates = useCrates();
  const membership = useCratesOfTrack(row.recordingId, libraryTrackId !== null);
  const remove = useRemoveFromCrate();
  const [removedFrom, setRemovedFrom] = useState<string | null>(null);
  const headingId = useId();
  // The line describes a removal that no longer stands once it is undone.
  const undone = useUndoneCount((state) => state.count);
  const [seenUndone, setSeenUndone] = useState(undone);
  if (seenUndone !== undone) {
    setSeenUndone(undone);
    setRemovedFrom(null);
  }

  if (libraryTrackId === null) return null;
  const list = crates.data;
  if (list === undefined || membership.data === undefined) {
    return membership.isError ? (
      <p role="alert" className={styles.error}>
        {errorMessage(membership.error)}
      </p>
    ) : null;
  }
  // In the order the tree is drawn.
  const inCrates = walk(list)
    .filter(({ crate }) => membership.data.includes(crate.id))
    .map(({ crate }) => ({ crate, label: pathOf(list, crate.id).join(PATH_SEPARATOR) }));

  return (
    <section aria-labelledby={headingId}>
      <h3 id={headingId} className={styles.heading}>
        {t("details.crates.title")}
      </h3>
      {inCrates.length === 0 ? (
        <p className={styles.quiet}>{t("details.crates.none")}</p>
      ) : (
        <ul className={styles.crates}>
          {inCrates.map(({ crate, label }) => (
            <li key={crate.id} className={styles.crateRow}>
              <Link
                id={`${headingId}-${crate.id}`}
                to="/crates"
                search={{ crate: crate.id }}
                className={styles.crateLink}
              >
                {label}
              </Link>
              <button
                type="button"
                className={styles.withdraw}
                aria-describedby={`${headingId}-${crate.id}`}
                disabled={remove.isPending}
                onClick={() => {
                  setRemovedFrom(null);
                  remove.mutate(
                    { id: crate.id, tracks: [libraryTrackId] },
                    { onSuccess: () => setRemovedFrom(label) },
                  );
                }}
              >
                {t("details.crates.remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {remove.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(remove.error)}
        </p>
      )}
      {removedFrom !== null && !remove.isError && (
        <p role="status" className={styles.quiet}>
          {t("details.crates.removed", { crate: removedFrom })}
        </p>
      )}
    </section>
  );
}
