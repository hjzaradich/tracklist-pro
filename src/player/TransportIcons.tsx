import type { ReactNode } from "react";
import styles from "./TransportIcons.module.css";

/*
 * The player bar's other icons, drawn like the play triangle
 * (`PlayPauseIcon`): in the text color, hidden from a screen reader, since
 * the button that holds each one carries the name.
 */

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg className={styles.icon} viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      {children}
    </svg>
  );
}

/** A bar and a triangle pointing back: the previous track. */
export function PreviousIcon() {
  return (
    <Icon>
      <path d="M3 2.5h2v11H3zM13 2.5v11L6 8z" />
    </Icon>
  );
}

/** A triangle pointing on and a bar: the next track. */
export function NextIcon() {
  return (
    <Icon>
      <path d="M11 2.5h2v11h-2zM3 2.5v11L10 8z" />
    </Icon>
  );
}

/** A short list with a play triangle after it: the tracks that play on in turn ("Play next"). */
export function PlayNextIcon() {
  return (
    <Icon>
      <path d="M2 3h8v1.5H2zM2 7h6v1.5H2zM2 11h6v1.5H2zM10.5 7v6l4.5-3z" />
    </Icon>
  );
}

/** A speaker; crossed out when the sound is off. */
export function VolumeIcon({ silent }: { silent: boolean }) {
  return (
    <Icon>
      <path d="M2 6h3l4-3.5v11L5 10H2z" />
      {silent ? (
        <path d="M11 6l4 4m0-4l-4 4" className={styles.stroke} />
      ) : (
        <path d="M11.5 5.5a3.5 3.5 0 0 1 0 5" className={styles.stroke} />
      )}
    </Icon>
  );
}
