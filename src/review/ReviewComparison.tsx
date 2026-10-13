import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type { ReviewComparison as Comparison, ReviewFileRow } from "../bindings";
import { Suggested } from "../suggest/Suggested";
import { reasonOptions, reasonProblem } from "../suggest/reasons";
import { trackLabel } from "./ReviewQueue";
import styles from "./ReviewComparison.module.css";

/** A duration as m:ss. */
function length(ms: number | null): string {
  if (ms === null) return "";
  const seconds = Math.round(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
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
 * the same columns. Read-only. The 1bE actions go under the table
 * (`ReviewActions`), and the "More details" columns (bitrate, sample rate,
 * date added) join the table: the rows already carry them.
 */
export function ReviewComparison({ comparison }: { comparison: Comparison }) {
  const { t } = useTranslation("review");
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
      <table className={styles.table} aria-label={t("compare.label")}>
        <thead>
          <tr>
            <th scope="col">{tLibrary("columns.file")}</th>
            {many && <th scope="col">{t("col.track")}</th>}
            <th scope="col">{t("col.format")}</th>
            <th scope="col">{t("col.quality")}</th>
            <th scope="col">{t("col.length")}</th>
            <th scope="col">{t("col.tags")}</th>
            <th scope="col">{t("col.cover")}</th>
            <th scope="col">{t("col.rekordbox")}</th>
          </tr>
        </thead>
        <tbody>
          {comparison.rows.map((row, n) => (
            <tr key={row.fileId ?? `entry-${n}`} className={styles.row}>
              <th scope="row" className={styles.file}>
                <span className={styles.name} title={row.path ?? row.name}>
                  {row.name}
                </span>
                <span className={styles.marks}>
                  {row.fileId !== null && row.fileId === comparison.suggestion?.what && (
                    <span>{t("mark.suggested")}</span>
                  )}
                  {row.linked && <span>{t("mark.linked")}</span>}
                  {row.source === "file" && !row.onDisk && <span className={styles.warn}>{tLibrary("fileMissing")}</span>}
                  {row.source === "file" && row.onDisk && !row.driveConnected && (
                    <span className={styles.warn}>{tLibrary("driveNotConnected")}</span>
                  )}
                </span>
              </th>
              {many && <td>{trackOf(row)}</td>}
              <td>{row.format?.toUpperCase() ?? ""}</td>
              <td className={styles.quality}>
                <Quality row={row} />
              </td>
              <td className={styles.number}>{length(row.durationMs)}</td>
              <td className={styles.number}>
                {row.source === "file" ? t("cell.tags", { filled: row.tagsFilled, total: row.tagsTotal }) : ""}
              </td>
              <td>{row.source === "file" ? (row.hasArt ? t("cell.yes") : t("cell.no")) : ""}</td>
              <td>{rekordboxCell(t, tAllMusic, row)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {comparison.moreFiles > 0 && (
        <p className={styles.more}>{t("compare.moreFiles", { count: comparison.moreFiles })}</p>
      )}
    </section>
  );
}
