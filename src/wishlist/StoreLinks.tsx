import { useState } from "react";
import { useTranslation } from "react-i18next";
import { STORES, storeLink, type Store } from "./linkTemplates";
import styles from "./wishlist.module.css";

type Track = { artist: string; title: string; version: string | null };

/**
 * A track's search address in each store, as text with a Copy link button
 * (ROADMAP 2.4). Nothing is opened: the app makes no network calls, so the
 * user pastes the address into their browser.
 */
export function StoreLinks({ track }: { track: Track }) {
  const { t } = useTranslation("wishlist");
  return (
    <details className={styles.links}>
      <summary>{t("links.title")}</summary>
      <ul className={styles.linkList}>
        {STORES.map((store) => (
          <StoreLink key={store} store={store} track={track} />
        ))}
      </ul>
    </details>
  );
}

function StoreLink({ store, track }: { store: Store; track: Track }) {
  const { t } = useTranslation("wishlist");
  const [state, setState] = useState<"copied" | "failed" | null>(null);
  const link = storeLink(store, track);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setState("copied");
    } catch {
      setState("failed");
    }
  };
  return (
    <li className={styles.linkRow} data-store={store}>
      <span className={styles.store}>{t(`store.${store}`)}</span>
      <code className={styles.url}>{link}</code>
      <button type="button" className={styles.action} onClick={() => void copy()}>
        {t("links.copy")}
      </button>
      {state !== null && (
        <span role="status" className={styles.muted}>
          {t(state === "copied" ? "links.copied" : "links.failed")}
        </span>
      )}
    </li>
  );
}
