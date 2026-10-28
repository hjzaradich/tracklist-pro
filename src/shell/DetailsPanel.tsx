import { useTranslation } from "react-i18next";
import { TrackDetails } from "./TrackDetails";
import { useSelectedTrack } from "./selectedTrack";
import styles from "./DetailsPanel.module.css";

/**
 * The Details panel on the right: the selected track's info (1cB-17): its
 * full title and path and where each tag value comes from.
 */
export function DetailsPanel({ className = "" }: { className?: string }) {
  const { t } = useTranslation("shell");
  const row = useSelectedTrack((state) => state.row);

  return (
    <aside
      aria-label={t("details.label")}
      className={`${styles.details} ${row === null ? styles.idle : ""} ${className}`}
    >
      {row === null ? (
        <p className={styles.empty}>{t("details.empty")}</p>
      ) : (
        <TrackDetails key={row.recordingId} row={row} />
      )}
    </aside>
  );
}
