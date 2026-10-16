import styles from "./PlayPauseIcon.module.css";

/**
 * The standard play triangle, or the pause bars when `playing`. Drawn in the
 * text color, so it follows the theme. It says nothing to a screen reader:
 * the button that holds it carries the name.
 */
export function PlayPauseIcon({ playing }: { playing: boolean }) {
  return (
    <svg className={styles.icon} viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      {playing ? (
        <path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z" />
      ) : (
        <path d="M4 2.5v11l9-5.5z" />
      )}
    </svg>
  );
}
