import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { LibraryTrack } from "../bindings";
import { describeNote, trackNotes } from "./libraryTrackNotes";
import styles from "./LibraryTrackList.module.css";

/**
 * The Library list (1aD-6): each Library track's title, artist and linked
 * file, in the order the backend sorted them. A track with no title shows
 * its file's name instead. Notes about a track (its file is missing, or sits somewhere fragile) sit
 * under the file's path.
 */
export function LibraryTrackList({
  tracks,
  onRemove,
}: {
  tracks: LibraryTrack[];
  onRemove: (track: LibraryTrack) => void;
}) {
  const { t } = useTranslation("library");
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th scope="col">{t("columns.title")}</th>
          <th scope="col">{t("columns.artist")}</th>
          <th scope="col">{t("columns.file")}</th>
          <td />
        </tr>
      </thead>
      <tbody>
        {tracks.map((track) => (
          <TrackRow key={track.id} track={track} onRemove={onRemove} />
        ))}
      </tbody>
    </table>
  );
}

function TrackRow({
  track,
  onRemove,
}: {
  track: LibraryTrack;
  onRemove: (track: LibraryTrack) => void;
}) {
  const { t } = useTranslation("library");
  const notesId = useId();
  const notes = trackNotes(track);
  return (
    <tr
      className={styles.row}
      data-file-present={track.file?.present ?? false}
      aria-describedby={notes.length > 0 ? notesId : undefined}
    >
      <td className={styles.title}>{track.title ?? track.file?.name}</td>
      <td>{track.artist}</td>
      <td className={styles.file}>
        <span className={styles.path}>{track.file?.path}</span>
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
      <td>
        <button type="button" className={styles.button} onClick={() => onRemove(track)}>
          {t("remove.button")}
        </button>
      </td>
    </tr>
  );
}
