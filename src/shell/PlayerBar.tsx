import { useTranslation } from "react-i18next";
import { formatTime } from "../player/formatTime";
import { PlayPauseIcon } from "../player/PlayPauseIcon";
import { usePlayer } from "../player/playerStore";
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
 * app. Play or pause, a scrub bar and the file's name; when the file can't
 * be played, a short line says why and nothing plays. Files are chosen with
 * a `PlayButton` in the lists.
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

  return (
    <section aria-label={t("shell:player.label")} className={`${styles.player} ${className}`}>
      {file === null ? (
        <p className={styles.empty}>{t("shell:player.empty")}</p>
      ) : (
        <>
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
          <span className={styles.name} title={file.name}>
            {file.name}
          </span>
          {status === "refused" && refusal !== null ? (
            <p role="status" className={styles.refusal}>
              {t(`player:${REFUSAL_LINES[refusal]}`)}
            </p>
          ) : (
            <>
              <input
                type="range"
                className={styles.scrub}
                aria-label={t("player:position")}
                min={0}
                max={duration}
                step="any"
                value={Math.min(position, duration)}
                disabled={status !== "ready"}
                onChange={(e) => seek(Number(e.target.value))}
              />
              <span className={styles.time}>
                {formatTime(position)} / {formatTime(duration)}
              </span>
            </>
          )}
        </>
      )}
    </section>
  );
}
