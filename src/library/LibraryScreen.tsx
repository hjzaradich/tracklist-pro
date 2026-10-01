import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { LibraryTrackList } from "./LibraryTrackList";
import styles from "./LibraryTrackList.module.css";
import { useLibraryTracks } from "./useLibraryTracks";

/**
 * The Library: the tracks the user added to play with (1aD-6). A plain
 * list for now; starting a Library and adding tracks arrive in 1aE.
 */
export function LibraryScreen() {
  const { t } = useTranslation("library");
  const tracks = useLibraryTracks();

  return (
    <StageScreen stage="library">
      {tracks.isError ? (
        <p role="alert" className={styles.error}>
          {errorMessage(tracks.error)}
        </p>
      ) : tracks.data === undefined ? null : tracks.data.length === 0 ? (
        <EmptyState>{t("empty")}</EmptyState>
      ) : (
        <div className={styles.scroll}>
          <LibraryTrackList tracks={tracks.data} />
        </div>
      )}
    </StageScreen>
  );
}
