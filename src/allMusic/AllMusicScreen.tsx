import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { AllMusicList } from "./AllMusicList";
import styles from "./AllMusicList.module.css";
import { useAddToLibrary, useAllMusic } from "./useAllMusic";

/**
 * All music, minimal (1aE-4): the tracks the scan found, with a search and
 * "Add to Library" per track. It's how a Library started fresh gets its
 * tracks. The full track browser is Phase 1c.
 */
export function AllMusicScreen() {
  const { t } = useTranslation("allMusic");
  const [search, setSearch] = useState("");
  const list = useAllMusic(search);
  const add = useAddToLibrary();
  const failure = list.error ?? add.error;

  return (
    <StageScreen stage="allMusic">
      <div className={styles.screen}>
        <input
          type="search"
          className={styles.search}
          aria-label={t("search")}
          placeholder={t("search")}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        {failure && (
          <p role="alert" className={styles.error}>
            {errorMessage(failure)}
          </p>
        )}
        {list.data === undefined ? null : list.data.tracks.length === 0 ? (
          <EmptyState>{search.trim() === "" ? t("empty") : t("noMatch")}</EmptyState>
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
