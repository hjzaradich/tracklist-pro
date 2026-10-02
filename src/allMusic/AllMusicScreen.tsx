import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useWaitingOn } from "../activity/useWaitingOn";
import { errorMessage } from "../api/errors";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { AllMusicList } from "./AllMusicList";
import styles from "./AllMusicList.module.css";
import { useAddToLibrary, useAllMusic } from "./useAllMusic";
import { after, useDebounced, type Schedule } from "./useDebounced";

/** How long the search waits for typing to stop before it asks the backend. */
export const SEARCH_DELAY_MS = 200;
const AFTER_TYPING = after(SEARCH_DELAY_MS);
/** The background work that puts tracks in All music. */
const FILLS_ALL_MUSIC = ["scan", "read", "hash", "group"] as const;

/**
 * All music, minimal (1aE-4): the tracks the scan found, with a search and
 * "Add to Library" per track. It's how a Library started fresh gets its
 * tracks. The full track browser is Phase 1c.
 *
 * The search asks the backend once typing has stopped (`schedule`; tests
 * pass their own), not on every keystroke: each search reads the whole
 * collection.
 */
export function AllMusicScreen({ schedule = AFTER_TYPING }: { schedule?: Schedule }) {
  const { t } = useTranslation("allMusic");
  const [typed, setTyped] = useState("");
  const search = useDebounced(typed, schedule);
  const list = useAllMusic(search);
  const add = useAddToLibrary();
  const failure = list.error ?? add.error;
  // Tracks exist once a scan has read, checked and grouped its files.
  const waiting = useWaitingOn(FILLS_ALL_MUSIC);

  return (
    <StageScreen stage="allMusic">
      <div className={styles.screen}>
        <input
          type="search"
          className={styles.search}
          aria-label={t("search")}
          placeholder={t("search")}
          value={typed}
          onChange={(event) => setTyped(event.target.value)}
        />
        {failure && (
          <p role="alert" className={styles.error}>
            {errorMessage(failure)}
          </p>
        )}
        {list.data === undefined ? null : list.data.tracks.length === 0 ? (
          <EmptyState>{search.trim() === "" ? (waiting ?? t("empty")) : t("noMatch")}</EmptyState>
        ) : (
          <>
            {list.data.total > list.data.tracks.length && (
              <p className={styles.showing}>
                {t("showing", { shown: list.data.tracks.length, total: list.data.total })}
              </p>
            )}
            <div className={styles.scroll}>
              <AllMusicList
                tracks={list.data.tracks}
                adding={add.isPending ? add.variables : undefined}
                onAdd={(recordingId) => add.mutate(recordingId)}
              />
            </div>
          </>
        )}
      </div>
    </StageScreen>
  );
}
