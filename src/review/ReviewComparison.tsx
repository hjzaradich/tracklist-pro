import { type KeyboardEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type { ReviewComparison as Comparison, ReviewFileRow } from "../bindings";
import { PlayButton } from "../player/PlayButton";
import { Suggested } from "../suggest/Suggested";
import { reasonOptions, reasonProblem } from "../suggest/reasons";
import { trackLabel } from "./ReviewQueue";
import { decidesTracks, ReviewActions } from "./ReviewActions";
import { itemKey } from "./useReview";
import styles from "./ReviewComparison.module.css";

/** A duration as m:ss. */
function length(ms: number | null): string {
  if (ms === null) return "";
  const seconds = Math.round(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** A sample rate in kHz: 44100 is "44.1 kHz". */
function sampleRate(t: TFunction<"review">, hz: number | null): string {
  return hz === null ? "" : t("cell.sampleRate", { khz: Math.round(hz / 100) / 10 });
}

/**
 * The time as an ISO time with its zone. SQLite writes "2026-09-30 10:00:00" in
 * UTC with no zone, which a bare Date would read as the viewer's local time.
 */
export function isoTime(added: string): string {
  return /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(added) ? added.replace(" ", "T") + "Z" : added;
}

/** The day a file was first seen, in the viewer's own format; the text itself if it isn't a date. */
export function addedOn(language: string, added: string | null): string {
  if (added === null) return "";
  const date = new Date(isoTime(added));
  return Number.isNaN(date.getTime())
    ? added
    : new Intl.DateTimeFormat(language, { dateStyle: "medium" }).format(date);
}

/** The quality column: the verdict, and the measured cutoff under it. */
function Quality({ row }: { row: ReviewFileRow }) {
  const { t } = useTranslation("review");
  if (row.source === "rekordboxEntry") return null;
  return (
    <>
      <span>{t(`verdict.${row.verdict ?? "notChecked"}`)}</span>
      {row.cutoffHz !== null && (
        <span className={styles.detail}>
          {t("cell.cutoff", { khz: Math.round(row.cutoffHz / 100) / 10 })}
        </span>
      )}
    </>
  );
}

/** Whether rekordbox uses the file: yes, no, or its match is only probable. */
function rekordboxCell(
  t: TFunction<"review">,
  tAllMusic: TFunction<"allMusic">,
  row: ReviewFileRow,
): string {
  if (row.source === "rekordboxEntry") return "";
  if (row.rekordbox === "probable") return tAllMusic("matchNotConfirmed");
  return row.rekordbox === "trusted" ? t("cell.yes") : t("cell.no");
}

/**
 * The comparison (1bD-2): the suggestion and why, then one row per file with
 * the same columns, then the actions for the item (`ReviewActions`, 1bE). The
 * "More details" switch (1bE-1) adds the columns the rows already carry:
 * bitrate, sample rate and date added. Whether it is on is the screen's
 * (`details`), so it stays on from one item to the next. Each file has a play button (1bE-4) for the
 * app's one player: switching from one row to another keeps the position
 * and whether it is playing (`player/playerStore.ts`). A rekordbox entry
 * with no file has none.
 *
 * On a possible version mix-up and on "Unclear duplicates" a file row can be
 * selected (1bE-2): a click on it, or Space while it has the focus. S then
 * splits that file off. No row is selected at first, and the selection
 * belongs to the item it was made on. Only the selected row is tinted, never
 * the playing one (docs/design-style.md).
 */
export function ReviewComparison({
  comparison,
  details,
  onDetails,
  onDecided,
}: {
  comparison: Comparison;
  /** "More details" is on. */
  details: boolean;
  onDetails: (on: boolean) => void;
  /** An action worked: the screen goes on to the next item. */
  onDecided?: () => void;
}) {
  const { t, i18n } = useTranslation("review");
  const { t: tLibrary } = useTranslation("library");
  const { t: tAllMusic } = useTranslation("allMusic");
  const { t: tSuggest } = useTranslation("suggest");
  // A reason's key comes from Rust, so it can't be typed as a literal key; it
  // was checked against the locale files at runtime (reasonProblem).
  const translate = tSuggest as unknown as (key: string, options: object) => string;
  const headingId = useId();
  const many = comparison.tracks.length > 1;
  const suggestedRow = comparison.suggestion
    ? comparison.rows.find((row) => row.fileId === comparison.suggestion?.what)
    : undefined;
  const trackOf = (row: ReviewFileRow): string => {
    if (row.source === "rekordboxEntry") return t("compare.rekordboxEntry");
    const track = comparison.tracks.find((tr) => tr.recordingId === row.recordingId);
    return track ? trackLabel(t, track) : "";
  };
  // The selected row's file, for the item it was selected on.
  const [picked, setPicked] = useState<{ item: string; file: number } | null>(null);
  const key = itemKey(comparison.id);
  const selectable = decidesTracks(comparison.kind);
  const selectedFile =
    selectable &&
    picked !== null &&
    picked.item === key &&
    comparison.rows.some((row) => row.fileId === picked.file)
      ? picked.file
      : null;
  // Only reasons the locale files can word are shown.
  const doubt = comparison.doubt.filter((reason) => reasonProblem([reason]) === null);
  const separator = tSuggest("separator");

  return (
    <section className={styles.comparison} aria-labelledby={headingId}>
      <h2 id={headingId} className={styles.heading}>
        {comparison.tracks.map((track, n) => (
          <span key={n} className={styles.headingTrack}>
            {trackLabel(t, track)}
          </span>
        ))}
      </h2>
      {comparison.suggestion && suggestedRow && (
        <Suggested suggestion={comparison.suggestion}>
          <strong>{t("compare.suggested", { name: suggestedRow.name })}</strong>
        </Suggested>
      )}
      {doubt.length > 0 && (
        <p className={styles.doubt}>
          <span className={styles.doubtLabel}>{t("compare.doubt")} </span>
          {doubt
            .map((reason) => translate(reason.key, reasonOptions(reason)))
            .join(separator)}
        </p>
      )}
      <label className={styles.details}>
        <input
          type="checkbox"
          checked={details}
          onChange={(event) => onDetails(event.target.checked)}
        />
        {t("compare.moreDetails")}
      </label>
      <div className={styles.tableScroll}>
      <table className={styles.table} aria-label={t("compare.label")}>
        <thead>
          <tr>
            <th scope="col">{tLibrary("columns.file")}</th>
            {many && (
              <th scope="col" className={styles.trackCol}>
                {t("col.track")}
              </th>
            )}
            <th scope="col">{t("col.format")}</th>
            <th scope="col">{t("col.quality")}</th>
            <th scope="col">{t("col.length")}</th>
            <th scope="col">{t("col.tags")}</th>
            <th scope="col">{t("col.cover")}</th>
            <th scope="col">{t("col.rekordbox")}</th>
            {details && (
              <>
                <th scope="col">{t("col.bitrate")}</th>
                <th scope="col">{t("col.sampleRate")}</th>
                <th scope="col">{t("col.added")}</th>
              </>
            )}
          </tr>
        </thead>
        <tbody>
          {comparison.rows.map((row, n) => (
            <tr
              key={row.fileId ?? `entry-${n}`}
              className={selectable ? `${styles.row} ${styles.selectable}` : styles.row}
              {...(selectable && row.fileId !== null
                ? {
                    "aria-selected": row.fileId === selectedFile,
                    tabIndex: 0,
                    onClick: () => setPicked({ item: key, file: row.fileId! }),
                    onKeyDown: (event: KeyboardEvent) => {
                      if (event.key !== " " || event.target !== event.currentTarget) return;
                      event.preventDefault();
                      setPicked({ item: key, file: row.fileId! });
                    },
                  }
                : {})}
            >
              <th scope="row" className={styles.file}>
                {/* The play button and the name share one line, so a row is no taller for it. */}
                <span className={styles.nameLine}>
                  {row.fileId !== null && (
                    <span className={styles.play}>
                      <PlayButton file={{ id: row.fileId, name: row.name }} />
                    </span>
                  )}
                  <span className={styles.name} title={row.path ?? row.name}>
                    {row.name}
                  </span>
                </span>
                <span className={styles.marks}>
                  {row.fileId !== null && row.fileId === comparison.suggestion?.what && (
                    <span className={`${styles.mark} ${styles.markSuggested}`}>{t("mark.suggested")}</span>
                  )}
                  {row.linked && <span className={`${styles.mark} ${styles.markLinked}`}>{t("mark.linked")}</span>}
                  {row.source === "file" && !row.onDisk && <span className={styles.warn}>{tLibrary("fileMissing")}</span>}
                  {row.source === "file" && row.onDisk && !row.driveConnected && (
                    <span className={styles.warn}>{tLibrary("driveNotConnected")}</span>
                  )}
                </span>
              </th>
              {many && <td className={styles.trackCell}>{trackOf(row)}</td>}
              <td>{row.format?.toUpperCase() ?? ""}</td>
              <td className={styles.quality}>
                <Quality row={row} />
              </td>
              <td className={styles.number}>{length(row.durationMs)}</td>
              <td className={styles.tags}>
                {row.source === "file" ? t("cell.tags", { filled: row.tagsFilled, total: row.tagsTotal }) : ""}
              </td>
              <td>{row.source === "file" ? (row.hasArt ? t("cell.yes") : t("cell.no")) : ""}</td>
              <td>{rekordboxCell(t, tAllMusic, row)}</td>
              {details && (
                <>
                  <td className={styles.number}>
                    {row.bitrateKbps === null ? "" : t("cell.bitrate", { kbps: row.bitrateKbps })}
                  </td>
                  <td className={styles.number}>{sampleRate(t, row.sampleRateHz)}</td>
                  <td className={styles.number}>{addedOn(i18n.language, row.addedAt)}</td>
                </>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      </div>
      {/* The actions stay in view under the table, whatever its height. */}
      <div className={styles.dock}>
        <ReviewActions
          key={key}
          comparison={comparison}
          selectedFile={selectedFile}
          onDecided={onDecided}
        />
      </div>
      {comparison.moreFiles > 0 && (
        <p className={styles.more}>{t("compare.moreFiles", { count: comparison.moreFiles })}</p>
      )}
    </section>
  );
}
