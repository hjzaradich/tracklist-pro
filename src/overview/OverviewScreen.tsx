import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AfterSendPanel } from "../afterSend/AfterSendLists";
import { errorMessage } from "../api/errors";
import styles from "../firstRun/firstRun.module.css";
import { MusicFoldersStep } from "../firstRun/MusicFoldersStep";
import { RekordboxOffer } from "../firstRun/RekordboxOffer";
import { StartFresh } from "../firstRun/StartFresh";
import { useLibraryTracks } from "../library/useLibraryTracks";
import { XmlSourcePanel } from "../rekordbox/XmlSourcePanel";
import { SendChecklist } from "../send/SendChecklist";
import sendStyles from "../send/SendChecklist.module.css";
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
 *
 * Once the Library holds tracks, "Send to rekordbox" opens the guided send
 * in the Overview's place (1aF-1).
 */
export function OverviewScreen() {
  const { t } = useTranslation("overview");
  const { t: tFirstRun } = useTranslation("firstRun");
  const library = useLibraryTracks();
  const { t: tSend } = useTranslation("send");
  const firstRun = library.data?.length === 0;
  const [sending, setSending] = useState(false);

  if (sending) {
    return (
      <StageScreen stage="overview">
        <SendChecklist onClose={() => setSending(false)} afterSend={<AfterSendPanel />} />
      </StageScreen>
    );
  }

  // The panels keep their places whether or not the first-run ones show,
  // so the offer's summary stays up when the Library stops being empty.
  // Until the Library has loaded, neither heading shows, nor the offer
  // (its title depends on whether the Library is empty). If the Library
  // can't be read, the offer stays away too: adding to it would fail.
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
        {library.data !== undefined && !firstRun && (
          <button type="button" className={sendStyles.primary} onClick={() => setSending(true)}>
            {tSend("open")}
          </button>
        )}
        {firstRun && <MusicFoldersStep />}
        <XmlSourcePanel />
        {library.data !== undefined && <RekordboxOffer firstRun={firstRun} />}
        {firstRun && <StartFresh />}
      </div>
    </StageScreen>
  );
}
