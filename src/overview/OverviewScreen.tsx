import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AfterSendPanel } from "../afterSend/AfterSendLists";
import { errorMessage } from "../api/errors";
import styles from "../firstRun/firstRun.module.css";
import { MusicFoldersStep } from "../firstRun/MusicFoldersStep";
import { RekordboxOffer } from "../firstRun/RekordboxOffer";
import { StartFresh } from "../firstRun/StartFresh";
import { useLibraryTracks } from "../library/useLibraryTracks";
import { useAnswerRequest } from "../palette/requests";
import { XmlSourcePanel } from "../rekordbox/XmlSourcePanel";
import { useReviewCounts } from "../review/useReview";
import { SendChecklist } from "../send/SendChecklist";
import { sendAvailable } from "../send/sendAvailable";
import { useSendState } from "../send/useSend";
import { StageScreen } from "../shell/StageScreen";
import { Charts } from "./Charts";
import { OutsideStrip } from "./OutsideStrip";
import { OverviewHeader } from "./OverviewHeader";
import overviewStyles from "./overview.module.css";
import { ProgressLine } from "./ProgressLine";
import { TodoList } from "./TodoList";
import { useOverview } from "./useOverview";

/**
 * The Overview (ROADMAP 1.13). Its top half (1cB-11) is the header, with the
 * four numbers and the Send button, what's waiting for the DJ, and the
 * progress line. Then come the energy and genre bars and the strip for what's
 * outside the Library (1cB-12). Below them are where rekordbox's collection
 * comes from (1aB-11) and the offer to add rekordbox tracks to the Library,
 * shown after any read while there's something to add (1aE-2).
 *
 * While the Library is empty it's also the first-run flow (1aE-1 to 1aE-4):
 * pick music folders, then start from rekordbox (the same offer) or start
 * fresh (add tracks from All music). An empty Library is all that decides
 * it: nothing records how the Library was started.
 *
 * Once the Library holds tracks, "Send to rekordbox" opens the guided send
 * in the Overview's place (1aF-1; it never sends by itself), and the Music
 * folders panel moves below the rest, so folders can still be added,
 * watched and scanned again.
 */
export function OverviewScreen() {
  const { t: tFirstRun } = useTranslation("firstRun");
  const library = useLibraryTracks();
  const firstRun = library.data?.length === 0;
  const tracks = library.data?.length;
  // The numbers wait for a Library with tracks; the command is slow on a big one.
  const overview = useOverview(tracks);
  const review = useReviewCounts();
  const { data: sendState } = useSendState(null);
  const [sending, setSending] = useState(false);
  // "Manage folders" in the strip takes the user down to the folders panel.
  const folders = useRef<HTMLDivElement>(null);
  // The command palette's "Send to rekordbox" (1cB-7) opens the same guided send.
  useAnswerRequest("send", () => {
    if (sendAvailable(tracks)) setSending(true);
  });

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
        ) : (
          // Once the Library has tracks, the header (with Send) leads instead.
          firstRun && <h2 className={styles.heading}>{tFirstRun("title")}</h2>
        )}
        {!firstRun && (library.data !== undefined || library.isError) && (
          <OverviewHeader
            libraryTracks={tracks}
            overview={overview.data}
            send={sendState}
            onSend={() => setSending(true)}
          />
        )}
        {!firstRun && library.data !== undefined && (
          <>
            <TodoList
              overview={overview.data}
              overviewError={overview.error}
              review={review.data}
              reviewError={review.error}
            />
            {overview.data && <ProgressLine overview={overview.data} />}
            {overview.data && <Charts overview={overview.data} />}
            {overview.data && (
              <OutsideStrip
                overview={overview.data}
                review={review.data}
                onManageFolders={() => folders.current?.focus()}
              />
            )}
          </>
        )}
        {firstRun && <MusicFoldersStep />}
        {/* Once the Library has tracks, Send is the Overview's one filled button. */}
        <XmlSourcePanel quiet={sendAvailable(tracks)} />
        {library.data !== undefined && <RekordboxOffer firstRun={firstRun} />}
        {firstRun && <StartFresh />}
        {library.data !== undefined && !firstRun && (
          <div ref={folders} tabIndex={-1} className={overviewStyles.anchor}>
            <MusicFoldersStep />
          </div>
        )}
      </div>
    </StageScreen>
  );
}
