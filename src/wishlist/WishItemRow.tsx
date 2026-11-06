import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { WishItem } from "../bindings";
import { StoreLinks } from "./StoreLinks";
import { reasonText } from "./wishlistText";
import styles from "./wishlist.module.css";

/**
 * One wishlist track: who and what it is, its result with the reason, the
 * Bought box, a note, and (for a track not owned) where to look for it.
 * Each change is one operation the top bar's Undo takes back.
 */
export function WishItemRow({
  item,
  busy,
  onBought,
  onNotes,
  onRemove,
}: {
  item: WishItem;
  busy: boolean;
  onBought: (bought: boolean) => void;
  onNotes: (notes: string | null) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation("wishlist");
  const [notes, setNotes] = useState(item.notes ?? "");
  const name = [item.artist, item.title, item.version].filter((part) => part).join(" - ");
  const owned = item.result.kind === "owned" || item.result.kind === "ownedOtherCut";
  const saveNotes = () => {
    const next = notes.trim() === "" ? null : notes;
    if (next !== item.notes) onNotes(next);
  };
  return (
    <li className={styles.item} data-kind={item.result.kind}>
      <div className={styles.itemMain}>
        <span className={styles.name}>{name}</span>
        <span className={styles.kind}>{t(`kind.${item.result.kind}`)}</span>
        <span className={styles.muted}>{reasonText(t, item.result.reason)}</span>
        <label className={styles.inline}>
          <input
            type="checkbox"
            checked={item.bought}
            disabled={busy}
            onChange={(event) => onBought(event.target.checked)}
          />
          {t("item.bought")}
        </label>
        <button type="button" className={styles.action} disabled={busy} onClick={onRemove}>
          {t("item.remove")}
        </button>
      </div>
      <label className={styles.inline}>
        {t("item.notes")}
        <input
          type="text"
          className={styles.notes}
          value={notes}
          disabled={busy}
          onChange={(event) => setNotes(event.target.value)}
          onBlur={saveNotes}
        />
      </label>
      {!owned && <StoreLinks track={item} />}
    </li>
  );
}
