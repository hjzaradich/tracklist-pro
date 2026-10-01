import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import styles from "../firstRun/firstRun.module.css";
import { MusicFoldersStep } from "../firstRun/MusicFoldersStep";
import { RekordboxOffer } from "../firstRun/RekordboxOffer";
import { StartFresh } from "../firstRun/StartFresh";
import { useLibraryTracks } from "../library/useLibraryTracks";
import { XmlSourcePanel } from "../rekordbox/XmlSourcePanel";
import { EmptyState, StageScreen } from "../shell/StageScreen";

/**
 * The Overview. Until its own features arrive (1c), it holds where
 * rekordbox's collection comes from (1aB-11) and the offer to add rekordbox
 * tracks to the Library, shown after any read while there's something to
 * add (1aE-2).
 *
 * While the Library is empty it's also the first-run flow (1aE-1 to 1aE-4):
 * pick music folders, then start from rekordbox (the same offer) or start
 * fresh (add tracks from All music). An empty Library is all that decides
 * it: nothing records how the Library was started.
 */
export function OverviewScreen() {
  const { t } = useTranslation("overview");
  const { t: tFirstRun } = useTranslation("firstRun");
  const library = useLibraryTracks();
  const firstRun = library.data?.length === 0;

  // The panels keep their places whether or not the first-run ones show,
  // so the offer's summary stays up when the Library stops being empty.
  // Until the Library has loaded, neither heading shows.
  return (
    <StageScreen stage="overview">
      <div className={styles.column}>
        {library.isError ? (
          <p role="alert" className={styles.problem}>
            {errorMessage(library.error)}
          </p>
        ) : library.data === undefined ? null : firstRun ? (
          <h2 className={styles.heading}>{tFirstRun("title")}</h2>
        ) : (
          <EmptyState>{t("empty")}</EmptyState>
        )}
        {firstRun && <MusicFoldersStep />}
        <XmlSourcePanel />
        <RekordboxOffer firstRun={firstRun} />
        {firstRun && <StartFresh />}
      </div>
    </StageScreen>
  );
}
