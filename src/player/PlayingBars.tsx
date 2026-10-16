import styles from "./PlayingBars.module.css";

/**
 * Three small bars that alternate in height while a track plays. Purely
 * decorative: no text and no data, and a screen reader never meets it (the
 * Pause button already says what is playing). It always takes its room, so
 * nothing moves when it appears; `playing` shows it.
 */
export function PlayingBars({ playing }: { playing: boolean }) {
  return (
    <span className={styles.bars} data-playing={playing} aria-hidden="true">
      <span className={styles.bar} />
      <span className={styles.bar} />
      <span className={styles.bar} />
    </span>
  );
}
