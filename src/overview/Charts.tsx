import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { Overview, TableFilter } from "../bindings";
import { LibraryDoor } from "./LibraryDoor";
import styles from "./overview.module.css";

/** How many genres get a bar of their own; the rest are added up as "Other". */
export const TOP_GENRES = 8;

/** One bar: what it counts and how many tracks. */
export interface Bar {
  /** Unique within its chart. */
  id: string;
  label: string;
  tracks: number;
  /** The name of the bar's link, which says what it opens (a door's only). */
  doorName?: string;
  /**
   * The Library's filter chip that holds the tracks the bar counts (1cC-4);
   * none for a bar that opens nothing.
   */
  filter?: TableFilter;
}

/**
 * The genre bars (ROADMAP 1.13): the biggest {@link TOP_GENRES} genres, most
 * tracks first, then "Other" (the rest added up) and "No genre". "Other" is
 * only made when it would add up two genres or more; one genre too many
 * keeps its own bar. A bar with no tracks isn't drawn.
 */
export function genreBars(
  overview: Pick<Overview, "genres" | "noGenre">,
  labels: { other: string; none: string },
): Bar[] {
  const { genres, noGenre } = overview;
  const grouped = genres.length > TOP_GENRES + 1;
  const own = grouped ? genres.slice(0, TOP_GENRES) : genres;
  const bars: Bar[] = own.map((genre) => ({ id: `genre:${genre.name}`, label: genre.name, tracks: genre.tracks }));
  if (grouped) {
    const other = genres.slice(TOP_GENRES).reduce((sum, genre) => sum + genre.tracks, 0);
    bars.push({ id: "other", label: labels.other, tracks: other });
  }
  if (noGenre > 0) bars.push({ id: "none", label: labels.none, tracks: noGenre });
  return bars;
}

/**
 * The energy bars: energy 1 to 10, always all ten, then "No energy" when a
 * track has none. Each is a door to the Library, filtered to that energy
 * (1cC-4). The genre bars are not doors: the Overview's genre is worked out in
 * Rust from the tags, and no filter of the Library's table can give the same
 * rows (see {@link genreBars}). (When no track has any energy the chart draws
 * none of them and says so instead: see {@link Charts}.)
 */
export function energyBars(
  overview: Pick<Overview, "energy" | "noEnergy">,
  labels: { none: string },
  doorName?: (level: number | null, tracks: number) => string,
): Bar[] {
  const bars: Bar[] = overview.energy.map((tracks, i) => ({
    id: `energy:${i + 1}`,
    label: String(i + 1),
    tracks,
    filter: { kind: "energy", level: i + 1 },
    doorName: doorName?.(i + 1, tracks),
  }));
  if (overview.noEnergy > 0) {
    bars.push({
      id: "none",
      label: labels.none,
      tracks: overview.noEnergy,
      filter: { kind: "noEnergy" },
      doorName: doorName?.(null, overview.noEnergy),
    });
  }
  return bars;
}

/**
 * The energy spread and the genres as one-color bar charts (ROADMAP 1.13).
 * Every bar has its label and its count written beside it, so no value
 * depends on the bar's color or length. The bars share one scale inside a
 * chart, the longest being the biggest count.
 */
export function Charts({ overview }: { overview: Overview }) {
  const { t } = useTranslation("overview");
  const hasEnergy = overview.energy.some((tracks) => tracks > 0);
  const genres = genreBars(overview, {
    other: t("charts.genres.other"),
    none: t("charts.genres.none"),
  });
  const hasGenre = overview.genres.length > 0;

  return (
    <div className={styles.charts}>
      <Chart
        title={t("charts.energy.title")}
        counting={overview.counting.index}
        empty={hasEnergy ? null : t("charts.energy.empty")}
        bars={energyBars(overview, { none: t("charts.energy.none") }, (level, count) =>
          level === null ? t("charts.energy.noneDoor", { count }) : t("charts.energy.door", { level, count }),
        )}
      />
      <Chart
        title={t("charts.genres.title")}
        counting={overview.counting.files || overview.counting.rekordbox}
        empty={hasGenre ? null : t("charts.genres.empty")}
        bars={genres}
      />
    </div>
  );
}

function Chart({
  title,
  counting,
  empty,
  bars,
}: {
  title: string;
  counting: boolean;
  /** The text that stands in for the bars when there is nothing to draw. */
  empty: string | null;
  bars: Bar[];
}) {
  const { t, i18n } = useTranslation("overview");
  const titleId = useId();
  const number = new Intl.NumberFormat(i18n.language);
  const most = Math.max(1, ...bars.map((bar) => bar.tracks));

  return (
    <section className={styles.chart} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.heading}>
        {title}
        {counting && <span className={styles.muted}> {t("todo.lineCounting")}</span>}
      </h2>
      {empty !== null ? (
        <p className={styles.muted}>{empty}</p>
      ) : (
        <ul className={styles.bars}>
          {bars.map((bar) => {
            const cells = (
              <>
                <span className={styles.barLabel} title={bar.label}>
                  {bar.label}
                </span>
                <span className={styles.barTrack} aria-hidden="true">
                  <span
                    className={styles.barFill}
                    style={{ width: `${(bar.tracks / most) * 100}%` }}
                  />
                </span>
                <span className={styles.barCount}>{number.format(bar.tracks)}</span>
              </>
            );
            return (
              <li key={bar.id} className={styles.bar}>
                {bar.filter !== undefined && bar.tracks > 0 ? (
                  <LibraryDoor filter={bar.filter} className={styles.barDoor} name={bar.doorName}>
                    {cells}
                  </LibraryDoor>
                ) : (
                  cells
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
