import { Link } from "@tanstack/react-router";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Overview, ReviewCounts } from "../bindings";
import styles from "./overview.module.css";
import { isCounting } from "./useOverview";

/** Where a line goes: Review for decisions, the Library for the rest (1cC-4 filters it). */
type Door = "/review" | "/library";

interface Line {
  id: string;
  count: number;
  door: Door;
  /** The number can still change because background work is running. */
  counting: boolean;
}

/** The Review groups, in the order Review shows them. */
const REVIEW_GROUPS = ["needsCloserLook", "clearBestFile", "qualityProblems", "conflicts"] as const;

/**
 * What's waiting for the DJ (ROADMAP 1.13), one line per thing, from the
 * numbers the Overview and Review already count. A line with nothing waiting
 * isn't drawn; each names what waits, with its count, and opens where it's
 * handled.
 */
export function waitingLines(overview: Overview | undefined, review: ReviewCounts | undefined): Line[] {
  const lines: Line[] = [];
  if (review !== undefined) {
    for (const group of REVIEW_GROUPS) {
      lines.push({ id: `review.${group}`, count: review[group], door: "/review", counting: false });
    }
  }
  if (overview !== undefined) {
    const { counting } = overview;
    lines.push(
      { id: "lowQuality", count: overview.lowQuality, door: "/library", counting: counting.quality },
      { id: "missingTags", count: overview.missingTags, door: "/library", counting: counting.files },
      { id: "missingArt", count: overview.missingArt, door: "/library", counting: counting.files },
      // Before rekordbox has been read, every track is "not analyzed", which says nothing.
      {
        id: "notAnalyzed",
        count: overview.rekordboxRead ? overview.notAnalyzed : 0,
        door: "/library",
        counting: counting.rekordbox,
      },
      { id: "inNoCrate", count: overview.inNoCrate, door: "/library", counting: false },
    );
  }
  return lines.filter((line) => line.count > 0);
}

export function TodoList({
  overview,
  overviewError,
  review,
  reviewError,
}: {
  overview: Overview | undefined;
  overviewError: unknown;
  review: ReviewCounts | undefined;
  reviewError: unknown;
}) {
  const { t } = useTranslation(["overview", "review", "conflicts"]);
  const titleId = useId();
  const lines = waitingLines(overview, review);
  const failed = overviewError ?? reviewError;
  // The list is complete once both counts are in and nothing is still being counted.
  const complete = overview !== undefined && review !== undefined && !isCounting(overview);

  const label = (line: Line) => {
    if (line.id.startsWith("review.")) {
      const group = line.id.slice("review.".length);
      return t("overview:todo.review", {
        count: line.count,
        group: group === "conflicts" ? t("conflicts:group") : t(`review:group.${group}` as "review:group.needsCloserLook"),
      });
    }
    return t(`overview:todo.${line.id}` as "overview:todo.lowQuality", { count: line.count });
  };

  return (
    <section className={styles.todo} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.heading}>
        {t("overview:todo.title")}
      </h2>
      {lines.length > 0 && (
        <ul className={styles.lines}>
          {lines.map((line) => (
            <li key={line.id}>
              <Link to={line.door} className={styles.line}>
                {label(line)}
                {line.counting && (
                  <span className={styles.muted}> {t("overview:todo.lineCounting")}</span>
                )}
              </Link>
            </li>
          ))}
        </ul>
      )}
      {complete && lines.length === 0 && <p>{t("overview:todo.empty")}</p>}
      {!complete && !failed && <p role="status">{t("overview:todo.counting")}</p>}
      {failed ? (
        <p role="alert" className={styles.problem}>
          {errorMessage(failed)}
        </p>
      ) : null}
    </section>
  );
}
