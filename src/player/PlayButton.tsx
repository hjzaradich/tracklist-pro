import { useTranslation } from "react-i18next";
import { usePlayer, type PlayerFile } from "./playerStore";
import styles from "./PlayButton.module.css";

/**
 * Plays `file` in the app's one player, or pauses it if it is the one
 * playing. Any list that shows a file can carry one. Choosing another file
 * while one plays carries the position over (see `playerStore.ts`).
 */
export function PlayButton({ file }: { file: PlayerFile }) {
  const { t } = useTranslation("player");
  const current = usePlayer((s) => s.file?.id === file.id);
  const playing = usePlayer((s) => s.playing);
  const refused = usePlayer((s) => s.status === "refused");
  const select = usePlayer((s) => s.select);
  const toggle = usePlayer((s) => s.toggle);
  // A file that can't be played isn't playing, whatever the intent is.
  const pause = current && playing && !refused;
  return (
    <button
      type="button"
      className={styles.button}
      data-play-for={file.id}
      aria-label={t(pause ? "pauseNamed" : "playNamed", { name: file.name })}
      // Pressing a refused file again asks again.
      onClick={() => (current && !refused ? toggle() : select(file))}
    >
      {t(pause ? "pause" : "play")}
    </button>
  );
}
