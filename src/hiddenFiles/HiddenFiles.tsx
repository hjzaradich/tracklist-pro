import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { errorMessage } from "../api/errors";
import type { HiddenRow, SortColumn, TableSort } from "../bindings";
import { nextSort } from "../trackTable/TrackTableView";
import styles from "./HiddenFiles.module.css";
import { useHiddenFiles, useShowAgain, useUseThisFile } from "./useHiddenFiles";

/** The columns the list can be sorted by. */
const SORTABLE = ["title", "artist", "file"] as const satisfies readonly SortColumn[];

/**
 * The Hidden filter in All music (1cB-5; ROADMAP 1.11): everything the app
 * isn't showing, one row per file, each with its reason and, where there is
 * one, its way back. A window of rows at a time, searched and sorted by the
 * backend like the track table.
 *
 * - "Show again" is for a file the user hid. Undo (top bar) hides it again.
 * - An extra copy says which file is its track's best. "Use this file
 *   instead" (1cB-22) makes it the track's file; Undo (top bar) takes that
 *   back. A Library track's extra copy has the reason and no button.
 * - A cleared Inbox file's way back, "Put back in Inbox", arrives with the
 *   Inbox (Phase 2).
 */
export function HiddenFiles({ search }: { search: string }) {
  const { t } = useTranslation("hiddenFiles");
  const { t: tt } = useTranslation("trackTable");
  const [sort, setSort] = useState<TableSort | null>(null);
  const list = useHiddenFiles(search, sort);
  const showAgain = useShowAgain();
  const useThis = useUseThisFile();
  const actionError = showAgain.error ?? useThis.error;
  const busy = showAgain.isPending || useThis.isPending;

  if (list.error != null) {
    return (
      <p role="alert" className={styles.error}>
        {errorMessage(list.error)}
      </p>
    );
  }
  if (list.total === undefined || (list.total === 0 && list.behind)) {
    return <p className={styles.empty}>{tt("updating")}</p>;
  }
  if (list.total === 0) {
    return <p className={styles.empty}>{search.trim() === "" ? t("empty") : t("noMatch")}</p>;
  }

  return (
    <div className={styles.view}>
      <p className={styles.count}>
        <span>{t("count", { count: list.total })}</span>
        {list.behind && <span>{tt("updating")}</span>}
      </p>
      {actionError != null && (
        <p role="alert" className={styles.error}>
          {errorMessage(actionError)}
        </p>
      )}
      <div className={styles.scroll}>
        <table className={styles.table} aria-label={t("label")}>
          <thead>
            <tr>
              {SORTABLE.map((column) => {
                const sorted = sort?.column === column;
                return (
                  <th
                    key={column}
                    scope="col"
                    aria-sort={sorted ? (sort.descending ? "descending" : "ascending") : undefined}
                  >
                    <button
                      type="button"
                      className={styles.sortButton}
                      title={tt("sortBy", { column: tt(`columns.${column}`) })}
                      onClick={() => setSort(nextSort(sort, column))}
                    >
                      {tt(`columns.${column}`)}
                    </button>
                  </th>
                );
              })}
              <th scope="col">{t("columns.reason")}</th>
              <th scope="col">
                <span className={styles.srOnly}>{tt("columns.actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {list.rows.map((row) => (
              <tr key={row.fileId}>
                <td>{row.track}</td>
                <td>{row.artist}</td>
                <td>{row.file}</td>
                <td className={styles.reason}>{reasonOf(row, t)}</td>
                <td className={styles.actions}>
                  {row.wayBack === "showAgain" && (
                    <button
                      type="button"
                      className={styles.action}
                      disabled={busy}
                      onClick={() => showAgain.mutate(row.fileId)}
                    >
                      {t("showAgain")}
                    </button>
                  )}
                  {row.wayBack === "useThisFile" && (
                    <button
                      type="button"
                      className={styles.action}
                      disabled={busy}
                      onClick={() => useThis.mutate(row.fileId)}
                    >
                      {t("useThisFile")}
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {list.hasMore && (
        <button
          type="button"
          className={styles.action}
          disabled={list.loadingMore}
          onClick={list.loadMore}
        >
          {t("showMore")}
        </button>
      )}
    </div>
  );
}

/** The reason for a row, in plain words. */
function reasonOf(row: HiddenRow, t: TFunction<"hiddenFiles">): string {
  switch (row.why) {
    case "userHid":
      return t("reason.userHid");
    case "clearedInbox":
      return t("reason.clearedInbox");
    case "extraFile":
      if (row.bestFile === null) return t("reason.extraFile", { track: row.track });
      if (row.trackInLibrary) {
        return t("reason.extraFileInLibrary", { track: row.track, best: row.bestFile });
      }
      return row.bestFileChosen
        ? t("reason.extraFileChosen", { track: row.track, best: row.bestFile })
        : t("reason.extraFileNamed", { track: row.track, best: row.bestFile });
  }
}
