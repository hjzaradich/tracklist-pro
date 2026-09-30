import { useTranslation } from "react-i18next";
import type { MusicFolder } from "../bindings";
import styles from "./MusicFolderWatchSwitch.module.css";

/**
 * The per-folder watcher toggle (1aC-6). On, the folder is rescanned in
 * the background whenever files change under it. The choice is stored by
 * the caller (`useSetMusicFolderWatch`).
 */
export function MusicFolderWatchSwitch({
  folder,
  onChange,
  disabled = false,
  describedBy,
}: {
  folder: MusicFolder;
  onChange: (watch: boolean) => void;
  disabled?: boolean;
  /** The id of the element naming the folder, so each row's checkbox is told apart. */
  describedBy?: string;
}) {
  const { t } = useTranslation("musicFolderWatch");
  return (
    <label className={styles.label}>
      <input
        type="checkbox"
        checked={folder.watch}
        disabled={disabled}
        aria-describedby={describedBy}
        onChange={(event) => onChange(event.target.checked)}
      />
      {t("watch")}
    </label>
  );
}
