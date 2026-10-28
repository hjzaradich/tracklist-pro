import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { errorMessage } from "../api/errors";
import type { HiddenRow, SortColumn, TableSort } from "../bindings";
import { nextSort } from "../trackTable/TrackTableView";
import styles from "./HiddenFiles.module.css";
import { useHiddenFiles, useShowAgain } from "./useHiddenFiles";

/** The columns the list can be sorted by. */
const SORTABLE = ["title", "artist", "file"] as const satisfies readonly SortColumn[];

/**
 * The Hidden filter in All music (1cB-5; ROADMAP 1.11): everything the app
 * isn't showing, one row per file, each with its reason and, where there is
 * one, its way back. A window of rows at a time, searched and sorted by the
 * backend like the track table.
 *
 * - "Show again" is for a file the user hid. Undo (top bar) hides it again.
 * - An extra copy says which file was chosen as its track's best. Its way
 *   back ("Use this file instead", ROADMAP 1.11) has to reopen the choice in
 *   Review, which can't reopen a settled choice yet: not offered.
 * - A cleared Inbox file's way back, "Put back in Inbox", arrives with the
 *   Inbox (Phase 2).
 */
export function HiddenFiles({ search }: { search: string }) {
  const { t } = useTranslation("hiddenFiles");
  const { t: tt } = useTranslation("trackTable");
  const [sort, setSort] = useState<TableSort | null>(null);
  const list = useHiddenFiles(search, sort);
  const showAgain = useShowAgain();

  if (list.error != null) {
    return (
      <p role="alert" className={styles.error}>
        {errorMessage(list.error)}
      </p>
    );
  }
  if (list.total === undefined) return null;
  if (list.total === 0) {
    if (list.behind) return null;
    return <p className={styles.empty}>{search.trim() === "" ? t("empty") : t("noMatch")}</p>;
  }

  return (
    <div className={styles.view}>
      <p className={styles.count}>
        <span>{t("count", { count: list.total })}</span>
        {list.behind && <span>{tt("updating")}</span>}
      </p>
      {showAgain.error != null && (
        <p role="alert" className={styles.error}>
          {errorMessage(showAgain.error)}
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
                      disabled={showAgain.isPending}
                      onClick={() => showAgain.mutate(row.fileId)}
                    >
                      {t("showAgain")}
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
      return row.bestFile === null
        ? t("reason.extraFile", { track: row.track })
        : t("reason.extraFileNamed", { track: row.track, best: row.bestFile });
  }
}
