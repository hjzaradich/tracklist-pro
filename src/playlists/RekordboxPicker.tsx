import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { CopiedFromRekordbox, RekordboxPlaylist } from "../bindings";
import { NameForm } from "../crates/CrateForms";
import styles from "../crates/CratesScreen.module.css";
import { PATH_SEPARATOR } from "../crates/tree";
import { usePlaylistFromRekordbox, useRekordboxPlaylistsToCopy } from "./usePlaylists";

/**
 * "Make an app playlist from a rekordbox playlist" (3C-P1b): the playlists of
 * the newest rekordbox read, one to pick, then the name the copy gets (the
 * playlist's own to start with). The copy is one step for Undo; entries whose
 * track isn't in your Library are left out, and the caller says how many.
 */
export function RekordboxPicker({
  onDone,
  onCancel,
}: {
  onDone: (copied: CopiedFromRekordbox, name: string) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("playlists");
  const found = useRekordboxPlaylistsToCopy(true);
  const copy = usePlaylistFromRekordbox();
  const [picked, setPicked] = useState<RekordboxPlaylist | null>(null);
  const list = found.data ?? [];
  return (
    <div className={styles.form} role="group" aria-label={t("fromRekordbox.label")}>
      {found.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(found.error)}
        </p>
      )}
      {found.data !== undefined && list.length === 0 && <p>{t("fromRekordbox.none")}</p>}
      {list.length > 0 && (
        <ul className={styles.list}>
          {list.map((playlist) => (
            <li key={playlist.path.join("\u0000")}>
              <button
                type="button"
                className={styles.crate}
                aria-pressed={playlist === picked}
                onClick={() => {
                  copy.reset();
                  setPicked(playlist);
                }}
              >
                <span className={styles.name}>{playlist.path.join(PATH_SEPARATOR)}</span>
                <span className={styles.count}>{t("fromRekordbox.entries", { count: playlist.entries })}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {copy.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(copy.error)}
        </p>
      )}
      {picked !== null ? (
        <NameForm
          key={picked.path.join("\u0000")}
          label={t("fromRekordbox.nameLabel")}
          confirm={t("fromRekordbox.confirm")}
          cancel={t("fromRekordbox.cancel")}
          initial={picked.path.at(-1) ?? ""}
          pending={copy.isPending}
          onSubmit={(name) =>
            copy.mutate({ path: picked.path, name }, { onSuccess: (copied) => onDone(copied, name.trim()) })
          }
          onCancel={onCancel}
        />
      ) : (
        <div className={styles.actions}>
          <button type="button" className={styles.button} onClick={onCancel}>
            {t("fromRekordbox.cancel")}
          </button>
        </div>
      )}
    </div>
  );
}
