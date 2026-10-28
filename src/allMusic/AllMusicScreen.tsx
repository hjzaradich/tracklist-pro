import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWaitingOn } from "../activity/useWaitingOn";
import { errorMessage } from "../api/errors";
import type { TableRow } from "../bindings";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { HiddenFiles } from "../hiddenFiles/HiddenFiles";
import { HiddenMatches } from "../hiddenFiles/HiddenMatches";
import { shownName } from "../trackTable/columns";
import { TrackTableView } from "../trackTable/TrackTableView";
import styles from "./AllMusicList.module.css";
import { LowBitrateWarning } from "./LowBitrateWarning";
import { ALL_MUSIC_TABLE_QUERY_KEY, useAddToLibrary } from "./useAllMusic";
import { after, useDebounced, type Schedule } from "./useDebounced";

/** How long the search waits for typing to stop before it asks the backend. */
export const SEARCH_DELAY_MS = 200;
const AFTER_TYPING = after(SEARCH_DELAY_MS);
/** The background work that puts tracks in All music. */
const FILLS_ALL_MUSIC = ["scan", "read", "hash", "group"] as const;

const ALL_MUSIC_SOURCE = { kind: "allMusic" } as const;

/**
 * All music (1aE-4): the tracks the scan found, in the track table (1cA-3)
 * with a search, filters and "Add to Library" per track. It's how a Library
 * started fresh gets its tracks.
 *
 * The search asks the backend once typing has stopped (`schedule`; tests
 * pass their own), not on every keystroke.
 *
 * "Hidden" switches the table to the Hidden filter (1cB-5): the files the
 * app isn't showing, with their reasons. A search that matches hidden files
 * whose tracks aren't in the results says so under the box, with "Show them"
 * (1cB-6), which opens the Hidden filter with the same search.
 */
export function AllMusicScreen({ schedule = AFTER_TYPING }: { schedule?: Schedule }) {
  const { t } = useTranslation("allMusic");
  const { t: th } = useTranslation("hiddenFiles");
  const [typed, setTyped] = useState("");
  const [showHidden, setShowHidden] = useState(false);
  const search = useDebounced(typed, schedule);
  const add = useAddToLibrary();
  // The add the backend asked the user to confirm (1bE-8), until they choose.
  const [warned, setWarned] = useState<{ recordingId: number; title: string; kbps: number } | null>(
    null,
  );
  // The track whose Add button gets focus back once the question is closed.
  const refocus = useRef<number | null>(null);
  const cancelWarning = useCallback(() => {
    refocus.current = warned?.recordingId ?? null;
    setWarned(null);
  }, [warned]);
  useEffect(() => {
    if (warned !== null || refocus.current === null) return;
    document.querySelector<HTMLElement>(`[data-add-for="${refocus.current}"]`)?.focus();
    refocus.current = null;
  }, [warned]);
  const addTrack = (row: TableRow) =>
    add.mutate(
      { recordingId: row.recordingId, confirmLowBitrate: false },
      {
        onSuccess: (outcome) => {
          if (outcome.outcome !== "needsConfirmation") return;
          setWarned({
            recordingId: row.recordingId,
            title: shownName(row),
            kbps: outcome.warning.kbps,
          });
        },
      },
    );
  // Tracks exist once a scan has read, checked and grouped its files.
  const waiting = useWaitingOn(FILLS_ALL_MUSIC);
  const adding = add.isPending ? add.variables.recordingId : warned?.recordingId;

  return (
    <StageScreen stage="allMusic">
      <div className={styles.screen}>
        <div className={styles.bar}>
          <input
            type="search"
            className={styles.search}
            aria-label={t("search")}
            placeholder={t("search")}
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
          />
          <button
            type="button"
            className={styles.toggle}
            aria-pressed={showHidden}
            onClick={() => setShowHidden((now) => !now)}
          >
            {th("filter")}
          </button>
        </div>
        {!showHidden && <HiddenMatches search={search} onShow={() => setShowHidden(true)} />}
        {add.error && (
          <p role="alert" className={styles.error}>
            {errorMessage(add.error)}
          </p>
        )}
        {warned !== null && (
          <LowBitrateWarning
            title={warned.title}
            kbps={warned.kbps}
            onAddAnyway={() => {
              add.mutate({ recordingId: warned.recordingId, confirmLowBitrate: true });
              setWarned(null);
            }}
            onCancel={cancelWarning}
          />
        )}
        {showHidden ? (
          <HiddenFiles search={search} />
        ) : (
          <TrackTableView
            screen="allMusic"
            source={ALL_MUSIC_SOURCE}
            keyPrefix={ALL_MUSIC_TABLE_QUERY_KEY}
            search={search}
            count={(total) => t("trackCount", { count: total })}
            empty={<EmptyState>{waiting ?? t("empty")}</EmptyState>}
            // The track left the list while the question was open: nothing to
            // ask about.
            onNarrowed={() => setWarned(null)}
            actionsWidth={220}
            actions={(row) =>
              row.inLibrary ? (
                <span className={styles.inLibrary}>{t("inLibrary")}</span>
              ) : (
                <>
                  <button
                    type="button"
                    className={styles.add}
                    data-add-for={row.recordingId}
                    disabled={adding !== undefined || row.matchNotConfirmed}
                    aria-describedby={row.matchNotConfirmed ? `why-${row.recordingId}` : undefined}
                    onClick={() => addTrack(row)}
                  >
                    {t("add")}
                  </button>
                  {row.matchNotConfirmed && (
                    <span id={`why-${row.recordingId}`} className={styles.inLibrary}>
                      {t("matchNotConfirmed")}
                    </span>
                  )}
                </>
              )
            }
          />
        )}
      </div>
    </StageScreen>
  );
}
