import { type KeyboardEvent, type PointerEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import styles from "./SeekBar.module.css";
import { formatTime } from "./formatTime";
import { timeAfterKey, timeAtPointer } from "./seekTime";
import { Waveform } from "./Waveform";

export interface SeekBarProps {
  /** The file whose waveform is drawn. */
  fileId: number;
  /** Where playback is, in seconds. */
  position: number;
  /** The file's length in seconds. */
  duration: number;
  /** Whether a seek can be made now (the file is loaded). */
  enabled: boolean;
  /** The user moved to this time, in seconds. */
  onSeek: (seconds: number) => void;
  className?: string;
}

/**
 * The waveform as the seek bar (1cB-10): a click on it moves to that time,
 * and the playhead follows the pointer while it is held down, moving the
 * audio when it is let go (so the sound isn't chased across the file with
 * every pixel). The arrow keys, Page Up and Page Down, Home and End move it
 * from the keyboard. Until the waveform is drawn, its placeholder stands in
 * and seeking works the same.
 */
export function SeekBar({ fileId, position, duration, enabled, onSeek, className = "" }: SeekBarProps) {
  const { t } = useTranslation("player");
  /** Where the pointer is, in seconds, while it is held down on the bar. */
  const [dragTo, setDragTo] = useState<number | null>(null);
  // A drag belongs to the file it began on, and to a bar that can seek: when
  // the file changes or the bar goes off, the release must not seek the new one.
  const [dragFor, setDragFor] = useState({ fileId, enabled });
  if (dragFor.fileId !== fileId || dragFor.enabled !== enabled) {
    setDragFor({ fileId, enabled });
    if (dragTo !== null) setDragTo(null);
  }
  const shown = dragTo ?? position;

  const timeAt = (event: PointerEvent<HTMLDivElement>): number => {
    const box = event.currentTarget.getBoundingClientRect();
    return timeAtPointer(event.clientX, box, duration);
  };
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!enabled || event.button !== 0) return;
    // Keeps the pointer's moves coming even when it leaves the bar. (Absent in a test's DOM.)
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDragTo(timeAt(event));
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (dragTo !== null) setDragTo(timeAt(event));
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (dragTo === null) return;
    setDragTo(null);
    onSeek(timeAt(event));
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!enabled || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    const to = timeAfterKey(event.key, position, duration);
    if (to === null) return;
    event.preventDefault();
    onSeek(to);
  };

  return (
    <div
      role="slider"
      tabIndex={enabled ? 0 : -1}
      aria-label={t("position")}
      aria-valuemin={0}
      aria-valuemax={duration}
      aria-valuenow={Math.floor(shown)}
      aria-valuetext={formatTime(shown)}
      aria-disabled={!enabled}
      className={`${styles.seek} ${className}`}
      data-enabled={enabled}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => setDragTo(null)}
      onLostPointerCapture={() => setDragTo(null)}
      onKeyDown={onKeyDown}
    >
      <Waveform fileId={fileId} position={shown} duration={duration} />
    </div>
  );
}
