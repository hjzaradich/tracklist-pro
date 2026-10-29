import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage, unwrap } from "../api/errors";
import { AddToCrateDialog } from "../crates/AddToCrateDialog";
import { type AddReport, useAddReport } from "../crates/addSelection";
import {
  commands,
  type SortColumn,
  type TableFilter,
  type TableRow,
  type TableSort,
  type TableSource,
} from "../bindings";
import { useReportSelection } from "../shell/selectedTrack";
import { useWhenUndone } from "../shell/undoneStore";
import { ColumnEditor } from "./ColumnEditor";
import { FilterChips } from "./FilterChips";
import { TrackTable } from "./TrackTable";
import styles from "./TrackTableView.module.css";
import type { TableScreen } from "./columns";
import { useSetEnergy } from "./useSetEnergy";
import { useTableColumns } from "./useTableColumns";
import { useTrackRows, type RowsQuery } from "./useTrackRows";

/**
 * The order a screen's table is in until a heading is clicked: the Library
 * list's (title, then artist) for the Library and All music, and the order
 * the tracks were added for a crate. `null` is the backend's own order.
 */
const DEFAULT_SORT: Readonly<Record<TableScreen, TableSort | null>> = {
  library: { column: "title", descending: false },
  allMusic: { column: "title", descending: false },
  crate: null,
};

/** The sort after `column`'s heading is clicked: its order the other way round if it is sorted already. */
export function nextSort(current: TableSort | null, column: SortColumn): TableSort {
  return current?.column === column
    ? { column, descending: !current.descending }
    : { column, descending: false };
}

/**
 * A screen's track table (1cA-3, 1cA-4): its filter chips, a line saying
 * how many tracks there are, and the table, fed a window of rows at a time
 * by the backend. The Library, All music and a crate differ only in what
 * they pass: where the tracks come from, their search text, their row
 * actions and what they say when there is nothing to list.
 *
 * Remember to give the view a `key` that changes with `source`, so the
 * sort and the filters start afresh for another crate.
 */
export function TrackTableView({
  screen,
  source,
  keyPrefix,
  search = "",
  count,
  empty,
  actions,
  actionsWidth,
  onSelectionChange,
  onNarrowed,
}: {
  screen: TableScreen;
  source: TableSource;
  /** The screen's query key; its lists' invalidations reach the windows under it. */
  keyPrefix: readonly unknown[];
  search?: string;
  /** The line over the table: how many tracks there are. */
  count: (total: number) => string;
  /** What to say when the screen has no tracks at all (nothing searched or filtered). */
  empty: ReactNode;
  actions?: (row: TableRow) => ReactNode;
  actionsWidth?: number;
  onSelectionChange?: (row: TableRow | null) => void;
  /** The tracks listed became other ones: another search, or a filter added or taken off (not a sort). */
  onNarrowed?: () => void;
}) {
  const { t } = useTranslation("trackTable");
  const [sort, setSort] = useState<TableSort | null>(null);
  const [filters, setFilters] = useState<TableFilter[]>([]);
  const choice = useTableColumns(screen);
  const reportSelection = useReportSelection(onSelectionChange);
  // How many tracks are selected, and the line saying what the energy keys last did.
  const [selectedCount, setSelectedCount] = useState(0);
  const [energyGiven, setEnergyGiven] = useState<{ energy: number; count: number } | null>(null);
  const giveEnergy = useSetEnergy();
  // The tracks "Add to crate or playlist" (the C key) is open for, and what
  // the last add to a crate did (1cB-13; the sidebar's drop reports the same way).
  const [adding, setAdding] = useState<readonly number[] | null>(null);
  const addReport = useAddReport((state) => state.report);
  // The line describes a change that stands: a move, another selection or an Undo drops it.
  const onSelectedChange = useCallback((selection: { count: number }) => {
    setSelectedCount(selection.count);
    setEnergyGiven(null);
    useAddReport.getState().clear();
  }, []);
  useWhenUndone(() => {
    setEnergyGiven(null);
    useAddReport.getState().clear();
  });
  // A line about a table that is gone doesn't wait for the next one.
  useEffect(() => () => useAddReport.getState().clear(), []);

  const query: RowsQuery = useMemo(
    () => ({ source, search: search.trim() === "" ? null : search, filters, sort }),
    [source, search, filters, sort],
  );
  const rows = useTrackRows(keyPrefix, query);
  const shownSort = sort ?? DEFAULT_SORT[screen];
  const narrowedBy = JSON.stringify([query.search, query.filters]);
  const seenNarrowing = useRef(narrowedBy);
  useEffect(() => {
    if (seenNarrowing.current === narrowedBy) return;
    seenNarrowing.current = narrowedBy;
    onNarrowed?.();
  }, [narrowedBy, onNarrowed]);
  const narrowed = filters.length > 0 || query.search !== null;

  // Position of a track in the rows, for the table to keep it selected when
  // the order or the filters change: the same source, search, filters and sort.
  const positionOf = (recordingId: number) =>
    unwrap(commands.trackTablePosition({ ...query, recordingId }));
  // The ids of rows first to last, for a range or all of the rows to select.
  const idsInRange = (first: number, last: number) =>
    unwrap(commands.trackTableIds({ ...query, offset: first, limit: last - first + 1, withTotal: false }));
  const onSetEnergy = (recordingIds: readonly number[], energy: number) =>
    giveEnergy.mutate(
      { recordingIds, energy },
      {
        // What changed, as the top bar's Undo counts it; nothing changed, nothing said.
        onSuccess: (done) => setEnergyGiven(done.changed > 0 ? { energy, count: done.changed } : null),
      },
    );

  if (rows.error != null) {
    return (
      <p role="alert" className={styles.error}>
        {errorMessage(rows.error)}
      </p>
    );
  }
  // Nothing to show until the first window says how many there are.
  if (rows.total === undefined || !choice.ready) return null;
  // While the index job is still writing the rows (after an upgrade, say),
  // zero rows means "not ready", never "no tracks": say so, not the empty state.
  if (rows.total === 0 && !narrowed && !rows.behind) return <>{empty}</>;

  return (
    <div className={styles.view}>
      <div className={styles.top}>
        <FilterChips
          screen={screen}
          source={source}
          keyPrefix={keyPrefix}
          filters={filters}
          onChange={setFilters}
        />
        <div className={styles.end}>
          <p className={styles.count}>
            <span>{count(rows.total)}</span>
            {selectedCount > 1 && <span>{t("selected", { selected: selectedCount })}</span>}
            {rows.behind && <span className={styles.updating}>{t("updating")}</span>}
            {energyGiven !== null && (
              <span role="status">{t("energySet", { energy: energyGiven.energy, count: energyGiven.count })}</span>
            )}
            {addReport !== null && <AddReportLine report={addReport} />}
            {giveEnergy.isError && (
              <span role="alert" className={styles.failed}>
                {errorMessage(giveEnergy.error)}
              </span>
            )}
          </p>
          <ColumnEditor
            screen={screen}
            columns={choice.columns}
            resized={Object.keys(choice.widths).length > 0}
            onChange={choice.change}
            onReset={choice.reset}
          />
        </div>
      </div>
      {rows.total === 0 ? (
        rows.behind ? null : <p className={styles.noMatch}>{t("noMatch")}</p>
      ) : (
        <TrackTable
          label={t("label")}
          columns={choice.columns}
          widths={choice.widths}
          onReorder={choice.change}
          onResize={choice.resize}
          total={rows.total}
          getRow={rows.getRow}
          onRangeChange={rows.setRange}
          sort={shownSort}
          onSort={(column) => setSort(nextSort(shownSort, column))}
          actions={actions}
          actionsWidth={actionsWidth}
          resetKey={JSON.stringify(query)}
          positionOf={positionOf}
          settled={!rows.loading}
          onSelectionChange={reportSelection}
          onSelectedChange={onSelectedChange}
          idsInRange={idsInRange}
          onSetEnergy={onSetEnergy}
          onAddToCrate={setAdding}
        />
      )}
      {adding !== null && <AddToCrateDialog recordings={adding} onClose={() => setAdding(null)} />}
    </div>
  );
}

/** What the last add to a crate did, in words (1cB-13). */
function AddReportLine({ report }: { report: AddReport }) {
  const { t } = useTranslation("crates");
  if (report.kind === "failed") {
    return (
      <span role="alert" className={styles.failed}>
        {report.message}
      </span>
    );
  }
  if (report.kind === "folder") return <span role="status">{t("addSelection.folder")}</span>;
  const notes = [
    report.alreadyThere > 0 ? t("addSelection.alreadyThere", { count: report.alreadyThere }) : null,
    report.outsideLibrary > 0 ? t("addSelection.outsideLibrary", { count: report.outsideLibrary }) : null,
  ].filter((note) => note !== null);
  const done = t("addSelection.added", { count: report.added, crate: report.crate });
  return <span role="status">{notes.length > 0 ? t("addSelection.withNotes", { done, notes: notes.join(", ") }) : done}</span>;
}
