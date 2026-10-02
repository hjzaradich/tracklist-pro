import { useTranslation } from "react-i18next";
import type { AllMusicTrack } from "../bindings";
import styles from "./AllMusicList.module.css";

/**
 * The minimal All music list (1aE-4): title, artist and file per track, in
 * the order the backend sorted them, and "Add to Library" for each track
 * that isn't in the Library. A track with no title shows its file's name.
 * A track that can't be added yet (its rekordbox match isn't confirmed)
 * has the button greyed out, with the reason beside it.
 * `adding` is the track an add is running for.
 */
export function AllMusicList({
  tracks,
  adding,
  onAdd,
}: {
  tracks: AllMusicTrack[];
  adding?: number;
  onAdd: (recordingId: number) => void;
}) {
  const { t } = useTranslation("allMusic");
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th scope="col">{t("columns.title")}</th>
          <th scope="col">{t("columns.artist")}</th>
          <th scope="col">{t("columns.file")}</th>
          <th scope="col">{t("columns.library")}</th>
        </tr>
      </thead>
      <tbody>
        {tracks.map((track) => (
          <tr key={track.recordingId} className={styles.row}>
            <td className={styles.title}>{track.title ?? track.file?.name}</td>
            <td>{track.artist}</td>
            <td className={styles.path}>{track.file?.path}</td>
            <td className={styles.library}>
              {track.inLibrary ? (
                <span className={styles.inLibrary}>{t("inLibrary")}</span>
              ) : (
                <>
                  <button
                    type="button"
                    className={styles.add}
                    disabled={adding !== undefined || track.matchNotConfirmed}
                    aria-describedby={track.matchNotConfirmed ? `why-${track.recordingId}` : undefined}
                    onClick={() => onAdd(track.recordingId)}
                  >
                    {t("add")}
                  </button>
                  {track.matchNotConfirmed && (
                    <span id={`why-${track.recordingId}`} className={styles.inLibrary}>
                      {t("matchNotConfirmed")}
                    </span>
                  )}
                </>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
