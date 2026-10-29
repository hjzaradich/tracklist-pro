import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import styles from "./TrackDragChip.module.css";
import { useTrackDrag } from "./trackDrag";

/**
 * The small label that follows the pointer while tracks are dragged (1cB-13):
 * how many are being carried. The drag leaves the text under it unselected
 * and the pointer showing a grab; the chip itself takes no pointer events, so
 * the crate underneath still hears them.
 */
export function TrackDragChip() {
  const { t } = useTranslation("crates");
  const ids = useTrackDrag((state) => state.ids);
  const x = useTrackDrag((state) => state.x);
  const y = useTrackDrag((state) => state.y);
  const dragging = ids !== null;
  useEffect(() => {
    if (!dragging) return;
    const { userSelect, cursor } = document.body.style;
    document.body.style.userSelect = "none";
    document.body.style.cursor = "grabbing";
    return () => {
      document.body.style.userSelect = userSelect;
      document.body.style.cursor = cursor;
    };
  }, [dragging]);
  if (ids === null) return null;
  return (
    <div className={styles.chip} style={{ left: x, top: y }} aria-hidden="true">
      {t("trackCount", { count: ids.length })}
    </div>
  );
}
