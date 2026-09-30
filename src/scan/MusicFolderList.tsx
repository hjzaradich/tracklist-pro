import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { MusicFolder } from "../bindings";
import { describeNote, folderNotes } from "./musicFolderStatus";
import styles from "./MusicFolderList.module.css";

/**
 * The music folders with what the scan knows about each: a folder whose
 * drive isn't connected is greyed out and says so (1aB-9); one the scan
 * couldn't fully read says how much it missed (1aB-14); online-only files
 * are counted (1aB-8). For the Music folders screen (1aE-1).
 */
export function MusicFolderList({ folders }: { folders: MusicFolder[] }) {
  const { t } = useTranslation("musicFolderStatus");
  if (folders.length === 0) return <p className={styles.empty}>{t("empty")}</p>;
  return (
    <ul className={styles.list}>
      {folders.map((folder) => (
        <FolderRow key={folder.id} folder={folder} />
      ))}
    </ul>
  );
}

function FolderRow({ folder }: { folder: MusicFolder }) {
  const { t } = useTranslation("musicFolderStatus");
  const notesId = useId();
  const notes = folderNotes(folder);
  return (
    <li
      className={styles.row}
      data-online={folder.online}
      aria-describedby={notes.length > 0 ? notesId : undefined}
    >
      <span className={styles.path}>{folder.path}</span>
      {notes.length > 0 && (
        <span id={notesId} className={styles.notes}>
          {notes.map((note) => (
            <span key={note.kind} className={styles.note} data-note={note.kind}>
              {describeNote(note, t)}
            </span>
          ))}
        </span>
      )}
    </li>
  );
}
