import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { WishlistOutcome } from "../bindings";
import stage from "../shell/StageScreen.module.css";
import { ImportPanel } from "./ImportPanel";
import { useWishItems, useWishlistAction, useWishlistFeature, useWishlists } from "./useWishlist";
import { refusalText } from "./wishlistText";
import { WishItemRow } from "./WishItemRow";
import styles from "./wishlist.module.css";

/**
 * Wishlists, at `/wishlists` (ROADMAP 2.3 and 2.4): make a list from pasted
 * text or a CSV file, see which tracks you own, and for the others copy a
 * store search link, tick Bought and keep a note. While the feature is off
 * (Settings), the screen says so and links to Settings.
 */
export function WishlistScreen() {
  const { t } = useTranslation("wishlist");
  const feature = useWishlistFeature();
  const on = feature.data === true;
  const lists = useWishlists(on);
  const [chosen, setChosen] = useState<number | null>(null);
  const items = useWishItems(on ? chosen : null);
  const action = useWishlistAction();
  // What the last change on the chosen list came to, when it was refused.
  const [refused, setRefused] = useState<WishlistOutcome | null>(null);

  const perform = (request: Parameters<typeof action.mutate>[0]) =>
    action.mutate(request, {
      onSuccess: (outcome) => {
        setRefused(outcome.outcome === "refused" ? outcome : null);
        if (request.kind === "delete" && outcome.outcome === "done") setChosen(null);
      },
    });

  const summaries = lists.data?.outcome === "lists" ? lists.data.lists : [];
  const rows = items.data?.outcome === "items" ? items.data.items : [];
  const failure = lists.error ?? items.error ?? action.error;

  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
      </div>
      <div className={stage.body}>
        {feature.data === false ? (
          <div>
            <p>{t("off.message")}</p>
            <Link to="/settings">{t("off.settings")}</Link>
          </div>
        ) : !on ? null : (
          <div className={styles.screen}>
            <ImportPanel onMade={setChosen} />
            {failure && (
              <p role="alert" className={styles.problem}>
                {errorMessage(failure)}
              </p>
            )}
            {refused?.outcome === "refused" && (
              <p role="alert" className={styles.problem}>
                {refusalText(t, refused.reason)}
              </p>
            )}
            <nav aria-label={t("lists.label")}>
              {summaries.length === 0 ? (
                <p className={styles.muted}>{t("lists.none")}</p>
              ) : (
                <ul className={styles.lists}>
                  {summaries.map((list) => (
                    <li key={list.id}>
                      <button
                        type="button"
                        className={styles.listButton}
                        aria-pressed={list.id === chosen}
                        onClick={() => setChosen(list.id)}
                      >
                        <span>{list.name}</span>
                        <span className={styles.muted}>
                          {t("lists.summary", {
                            count: list.total,
                            owned: list.owned,
                            cut: list.ownedOtherCut,
                            rework: list.rework,
                            missing: list.missing,
                            bought: list.bought,
                          })}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </nav>
            {chosen === null ? (
              <p className={styles.muted}>{t("items.choose")}</p>
            ) : (
              <section aria-label={t("items.label")}>
                <button
                  type="button"
                  className={styles.action}
                  disabled={action.isPending}
                  onClick={() => perform({ kind: "delete", wishlistId: chosen })}
                >
                  {t("lists.delete")}
                </button>
                {items.data !== undefined && rows.length === 0 && (
                  <p className={styles.muted}>{t("items.none")}</p>
                )}
                <ul className={styles.items}>
                  {rows.map((item) => (
                    <WishItemRow
                      key={item.id}
                      item={item}
                      busy={action.isPending}
                      onBought={(bought) => perform({ kind: "setBought", itemId: item.id, bought })}
                      onNotes={(notes) => perform({ kind: "setNotes", itemId: item.id, notes })}
                      onRemove={() => perform({ kind: "removeItem", itemId: item.id })}
                    />
                  ))}
                </ul>
              </section>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
