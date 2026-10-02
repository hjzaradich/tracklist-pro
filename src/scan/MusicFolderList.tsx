import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { MusicFolder, MusicFolderId } from "../bindings";
import { describeNote, folderNotes } from "./musicFolderStatus";
import { MusicFolderWatchSwitch } from "./MusicFolderWatchSwitch";
import styles from "./MusicFolderList.module.css";

/** Called when the user turns a folder's watcher on or off (1aC-6). */
export type OnWatchChange = (id: MusicFolderId, watch: boolean) => void;

/** Called when the user asks for a folder to be scanned again. */
export type OnScanAgain = (id: MusicFolderId) => void;

/**
 * The music folders with what the scan knows about each: a folder whose
 * drive isn't connected is greyed out and says so (1aB-9); one the scan
 * couldn't fully read says how much it missed (1aB-14); online-only files
 * are counted (1aB-8). With `onWatchChange`, each row also has its watcher
 * toggle (1aC-6); with `onScanAgain`, a "Scan again" button. For the Music folders screen (1aE-1).
 */
export function MusicFolderList({
  folders,
  onWatchChange,
  onScanAgain,
}: {
  folders: MusicFolder[];
  onWatchChange?: OnWatchChange;
  onScanAgain?: OnScanAgain;
}) {
  const { t } = useTranslation("musicFolderStatus");
  if (folders.length === 0) return <p className={styles.empty}>{t("empty")}</p>;
  return (
    <ul className={styles.list}>
      {folders.map((folder) => (
        <FolderRow
          key={folder.id}
          folder={folder}
          onWatchChange={onWatchChange}
          onScanAgain={onScanAgain}
        />
      ))}
    </ul>
  );
}

function FolderRow({
  folder,
  onWatchChange,
  onScanAgain,
}: {
  folder: MusicFolder;
  onWatchChange?: OnWatchChange;
  onScanAgain?: OnScanAgain;
}) {
  const { t } = useTranslation("musicFolderStatus");
  const notesId = useId();
  const pathId = useId();
  const notes = folderNotes(folder);
  return (
    <li
      className={styles.row}
      data-online={folder.online}
      aria-describedby={notes.length > 0 ? notesId : undefined}
    >
      <span id={pathId} className={styles.path}>
        {folder.path}
      </span>
      {onWatchChange && (
        <MusicFolderWatchSwitch
          folder={folder}
          describedBy={pathId}
          onChange={(watch) => onWatchChange(folder.id, watch)}
        />
      )}
      {onScanAgain && (
        // Greyed out while the drive is away: the row's note says so.
        <button
          type="button"
          aria-describedby={pathId}
          disabled={!folder.online}
          onClick={() => onScanAgain(folder.id)}
        >
          {t("scanAgain")}
        </button>
      )}
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
