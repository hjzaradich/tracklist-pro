import { Link } from "@tanstack/react-router";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { Overview, ReviewCounts } from "../bindings";
import styles from "./overview.module.css";

/**
 * "Outside your Library" (ROADMAP 1.13): the tracks in All music that are
 * not Library tracks, how many items in Review wait for a decision, and the
 * doors to Review, All music and the music folders below. All quiet links:
 * the Overview's one filled button is Send.
 */
export function OutsideStrip({
  overview,
  review,
  onManageFolders,
}: {
  overview: Overview;
  /** `undefined` until Review's count is in or if it can't be read. */
  review: ReviewCounts | undefined;
  onManageFolders: () => void;
}) {
  const { t } = useTranslation(["overview", "shell"]);
  const titleId = useId();
  return (
    <section className={styles.outside} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.heading}>
        {t("overview:outside.title")}
      </h2>
      <p>
        {t("overview:outside.tracks", { count: overview.outsideLibrary })}
        {overview.counting.files && (
          <span className={styles.muted}> {t("overview:todo.lineCounting")}</span>
        )}
      </p>
      {review !== undefined && (
        <p>{t("overview:outside.decisions", { count: review.total })}</p>
      )}
      <div className={styles.doors}>
        <Link to="/review" className={styles.door}>
          {t("shell:stages.review")}
        </Link>
        <Link to="/all-music" className={styles.door}>
          {t("shell:stages.allMusic")}
        </Link>
        <button type="button" className={styles.door} onClick={onManageFolders}>
          {t("overview:outside.manageFolders")}
        </button>
      </div>
    </section>
  );
}
