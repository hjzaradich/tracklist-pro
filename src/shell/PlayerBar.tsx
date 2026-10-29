import { useTranslation } from "react-i18next";
import { formatTime } from "../player/formatTime";
import { PlayingBars } from "../player/PlayingBars";
import { PlayPauseIcon } from "../player/PlayPauseIcon";
import { usePlayer } from "../player/playerStore";
import { SeekBar } from "../player/SeekBar";
import { canGoPrevious, hasRowBelow, playNext, playPrevious, useStepPlace } from "../player/stepList";
import { NextIcon, PreviousIcon, VolumeIcon } from "../player/TransportIcons";
import styles from "./PlayerBar.module.css";

/**
 * The line for each reason a file can't be played. Four lines for six
 * codes: a file nothing is known of, one the OS wouldn't read and one that
 * isn't audio the player can use all read the same.
 */
const REFUSAL_LINES = {
  missing: "refused.missing",
  drive_not_connected: "refused.driveNotConnected",
  online_only: "refused.onlineOnly",
  unknown_file: "refused.cantPlay",
  unreadable: "refused.cantPlay",
  not_decodable: "refused.cantPlay",
} as const;

/**
 * The preview player along the bottom (1.11): the one player for the whole
 * app. Previous and next track through the rows of the table on screen,
 * play or pause, the file's name, its waveform as the seek bar (1cB-9,
 * 1cB-10), the time, and volume and mute (kept per user). When the file
 * can't be played, a short line says why and nothing plays; the track can
 * still be stepped past. Files are chosen with a `PlayButton` in the lists.
 */
export function PlayerBar({ className = "" }: { className?: string }) {
  const { t } = useTranslation(["shell", "player"]);
  const file = usePlayer((s) => s.file);
  const status = usePlayer((s) => s.status);
  const refusal = usePlayer((s) => s.refusal);
  const position = usePlayer((s) => s.position);
  const duration = usePlayer((s) => s.duration);
  const playing = usePlayer((s) => s.playing);
  const toggle = usePlayer((s) => s.toggle);
  const seek = usePlayer((s) => s.seek);
  const volume = usePlayer((s) => s.volume);
  const muted = usePlayer((s) => s.muted);
  const setVolume = usePlayer((s) => s.setVolume);
  const toggleMute = usePlayer((s) => s.toggleMute);
  const total = useStepPlace((s) => s.total);
  const focus = useStepPlace((s) => s.focus);
  const place = { total, focus };
  const silent = muted || volume === 0;

  return (
    <section aria-label={t("shell:player.label")} className={`${styles.player} ${className}`}>
      {file === null ? (
        <p className={styles.empty}>{t("shell:player.empty")}</p>
      ) : (
        <>
          <button
            type="button"
            className={styles.step}
            disabled={!canGoPrevious(place, { status, position })}
            aria-label={t("player:previous")}
            title={t("player:previous")}
            onClick={playPrevious}
          >
            <PreviousIcon />
          </button>
          <button
            type="button"
            className={styles.toggle}
            disabled={status === "refused"}
            aria-label={t(playing ? "player:pause" : "player:play")}
            title={t(playing ? "player:pause" : "player:play")}
            onClick={toggle}
          >
            <PlayPauseIcon playing={playing} />
          </button>
          <button
            type="button"
            className={styles.step}
            disabled={!hasRowBelow(place)}
            aria-label={t("player:next")}
            title={t("player:next")}
            onClick={playNext}
          >
            <NextIcon />
          </button>
          <PlayingBars playing={playing && status === "ready"} />
          <span className={styles.name} title={file.name}>
            {file.name}
          </span>
          {status === "refused" && refusal !== null ? (
            <p role="status" className={styles.refusal}>
              {t(`player:${REFUSAL_LINES[refusal]}`)}
            </p>
          ) : (
            <>
              <SeekBar
                fileId={file.id}
                position={position}
                duration={duration}
                enabled={status === "ready"}
                onSeek={seek}
                className={styles.waveform}
              />
              <span className={styles.time}>
                {formatTime(position)} / {formatTime(duration)}
              </span>
            </>
          )}
          <button
            type="button"
            className={styles.step}
            aria-label={t(silent ? "player:unmute" : "player:mute")}
            title={t(silent ? "player:unmute" : "player:mute")}
            onClick={toggleMute}
          >
            <VolumeIcon silent={silent} />
          </button>
          <input
            type="range"
            className={styles.volume}
            aria-label={t("player:volume")}
            min={0}
            max={1}
            step={0.01}
            value={muted ? 0 : volume}
            onChange={(e) => setVolume(Number(e.target.value))}
          />
        </>
      )}
    </section>
  );
}
