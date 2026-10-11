import { type ReactNode, useId } from "react";
import { useTranslation } from "react-i18next";
import type { LibraryTrack } from "../bindings";
import { describeNote, trackNotes } from "./libraryTrackNotes";
import { shownTitle } from "./RemoveFromLibrary";
import styles from "./LibraryTrackList.module.css";

/**
 * The Library list (1aD-6): each Library track's title, artist and linked
 * file, in the order the backend gave them. A track with no title shows
 * its file's name instead. Notes about a track (its file is missing, or sits somewhere fragile) sit
 * under the file's path. `actions` fills the last cell of a row: the
 * Library screen puts "Add to crate" and Remove there, a crate Remove only.
 */
export function LibraryTrackList({
  tracks,
  actions,
}: {
  tracks: LibraryTrack[];
  actions: (track: LibraryTrack) => ReactNode;
}) {
  const { t } = useTranslation("library");
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th scope="col">{t("columns.title")}</th>
          <th scope="col">{t("columns.artist")}</th>
          <th scope="col">{t("columns.file")}</th>
          <th scope="col">
            <span className={styles.hidden}>{t("columns.actions")}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {tracks.map((track) => (
          <TrackRow key={track.id} track={track} actions={actions} />
        ))}
      </tbody>
    </table>
  );
}

function TrackRow({
  track,
  actions,
}: {
  track: LibraryTrack;
  actions: (track: LibraryTrack) => ReactNode;
}) {
  const { t } = useTranslation("library");
  const notesId = useId();
  const notes = trackNotes(track);
  // Cut off with "…" when it's long: the full text is the cell's `title`.
  const title = shownTitle(track);
  return (
    <tr
      className={styles.row}
      data-file-present={track.file?.present ?? false}
      aria-describedby={notes.length > 0 ? notesId : undefined}
    >
      <td className={styles.title} title={title}>
        {title}
      </td>
      <td className={styles.artist} title={track.artist ?? undefined}>
        {track.artist}
      </td>
      <td className={styles.file}>
        <span className={styles.path} title={track.file?.path}>
          {track.file?.path}
        </span>
        {notes.length > 0 && (
          <span id={notesId} className={styles.notes}>
            {notes.map((note) => (
              <span key={note.kind} className={styles.note} data-note={note.kind}>
                {describeNote(note, t)}
              </span>
            ))}
          </span>
        )}
      </td>
      <td className={styles.actionsCell}>{actions(track)}</td>
    </tr>
  );
}
