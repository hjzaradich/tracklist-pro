import { useId, useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { Waveform as WaveformAnswer } from "../bindings";
import styles from "./Waveform.module.css";
import { useWaveform } from "./useWaveform";
import { HEIGHT, outline, playedFraction } from "./waveformShape";

export interface WaveformPeaksProps {
  /** Each bucket's loudest sample, 0 to 255. */
  peaks: readonly number[];
  /** Each bucket's RMS level, 0 to 255, the same length as `peaks`. */
  levels: readonly number[];
  /** The length the waveform was drawn from, in milliseconds. */
  durationMs: number;
  /** Where playback is, in seconds. */
  position: number;
  /** The track's length in seconds, if the player knows it better. */
  duration?: number;
  className?: string;
}

/**
 * Draws a waveform that is already in hand, with the playhead at
 * `position`. It stretches to the width it is given: the buckets are the
 * same however wide it is.
 */
export function WaveformPeaks({
  peaks,
  levels,
  durationMs,
  position,
  duration,
  className = "",
}: WaveformPeaksProps) {
  const { t } = useTranslation("player");
  const gradient = `wave${useId().replace(/\W/g, "")}`;
  // Drawn once per waveform: the playhead only moves the gradient's edge.
  const peaksPath = useMemo(() => outline(peaks), [peaks]);
  const levelsPath = useMemo(() => outline(levels), [levels]);
  const length = duration !== undefined && duration > 0 ? duration : durationMs / 1000;
  const played = playedFraction(position, length);
  return (
    <div role="img" aria-label={t("waveform.label")} className={`${styles.wave} ${className}`}>
      <svg
        className={styles.svg}
        viewBox={`0 0 ${Math.max(peaks.length, 1)} ${HEIGHT}`}
        preserveAspectRatio="none"
        aria-hidden="true"
        focusable="false"
      >
        <defs>
          {/* A hard edge at the playhead: accent before it, muted after. */}
          <linearGradient id={gradient} x1="0" x2="1" y1="0" y2="0">
            <stop offset="0" className={styles.played} />
            <stop offset={played} className={styles.played} />
            <stop offset={played} className={styles.unplayed} />
            <stop offset="1" className={styles.unplayed} />
          </linearGradient>
        </defs>
        <path className={styles.peaks} d={peaksPath} fill={`url(#${gradient})`} />
        <path d={levelsPath} fill={`url(#${gradient})`} />
      </svg>
      <span className={styles.playhead} data-playhead style={{ left: `${played * 100}%` }} />
    </div>
  );
}

/** What stands in until there are peaks: a thin line, named for why. */
export function WaveformPlaceholder({
  unavailable = false,
  className = "",
}: {
  /** The file can't be decoded, so no waveform will come. */
  unavailable?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("player");
  return (
    <div
      role="img"
      aria-label={t(unavailable ? "waveform.unavailable" : "waveform.notReady")}
      className={`${styles.placeholder} ${className}`}
    />
  );
}

function isReady(
  answer: WaveformAnswer | undefined | null,
): answer is Extract<WaveformAnswer, { state: "ready" }> {
  return answer?.state === "ready";
}

export interface WaveformProps {
  /** The file whose waveform to show (a track's shown file); `null` shows the placeholder. */
  fileId: number | null;
  /** Where playback is, in seconds. */
  position: number;
  /** The track's length in seconds, if the player knows it. */
  duration?: number;
  className?: string;
}

/**
 * A file's waveform with a playhead (1cB-9). While the waveform job hasn't
 * reached the file, or the file can't be decoded, a placeholder stands in.
 * The player bar shows it for the file that is loaded; the full player bar
 * (1cB-10) builds on it.
 */
export function Waveform({ fileId, position, duration, className }: WaveformProps) {
  const { data } = useWaveform(fileId);
  if (isReady(data)) {
    return (
      <WaveformPeaks
        peaks={data.peaks}
        levels={data.levels}
        durationMs={data.durationMs}
        position={position}
        duration={duration}
        className={className}
      />
    );
  }
  return <WaveformPlaceholder unavailable={data?.state === "unavailable"} className={className} />;
}
