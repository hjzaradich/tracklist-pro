import { open } from "@tauri-apps/plugin-dialog";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { WishlistOutcome } from "../bindings";
import { refusalText, stemOf } from "./wishlistText";
import { useWishlistAction } from "./useWishlist";
import styles from "./wishlist.module.css";

/**
 * Makes a wishlist from a pasted tracklist or a CSV file (ROADMAP 2.3), then
 * says how many tracks it took and which lines it could not. `onMade` is
 * told the new list so the screen can open it.
 */
export function ImportPanel({ onMade }: { onMade: (wishlistId: number) => void }) {
  const { t } = useTranslation("wishlist");
  const titleId = useId();
  const [name, setName] = useState("");
  const [pasted, setPasted] = useState("");
  const [result, setResult] = useState<WishlistOutcome | null>(null);
  const action = useWishlistAction();

  const finish = (outcome: WishlistOutcome) => {
    setResult(outcome);
    if (outcome.outcome === "done") {
      if (outcome.wishlistId !== null) onMade(outcome.wishlistId);
      setPasted("");
    }
  };
  const makeFromPaste = () =>
    action.mutate({ kind: "createFromPaste", name, pasted }, { onSuccess: finish });
  const chooseCsv = async () => {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: t("import.csvFilter"), extensions: ["csv"] }],
    });
    if (typeof picked !== "string") return;
    const listName = name.trim() === "" ? stemOf(picked) : name;
    action.mutate({ kind: "createFromCsv", name: listName, path: picked }, { onSuccess: finish });
  };

  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.panelTitle}>
        {t("import.title")}
      </h2>
      <label className={styles.field}>
        {t("import.name")}
        <input type="text" value={name} onChange={(event) => setName(event.target.value)} />
      </label>
      <label className={styles.field}>
        {t("import.paste")}
        <textarea rows={6} value={pasted} onChange={(event) => setPasted(event.target.value)} />
      </label>
      <div className={styles.buttons}>
        <button
          type="button"
          className={styles.primary}
          disabled={action.isPending}
          onClick={makeFromPaste}
        >
          {t("import.makeFromPaste")}
        </button>
        <button
          type="button"
          className={styles.action}
          disabled={action.isPending}
          onClick={() => void chooseCsv()}
        >
          {t("import.chooseCsv")}
        </button>
      </div>
      <p className={styles.muted}>{t("import.csvNameHint")}</p>
      {action.isError && (
        <p role="alert" className={styles.problem}>
          {errorMessage(action.error)}
        </p>
      )}
      {result?.outcome === "refused" && (
        <p role="alert" className={styles.problem}>
          {refusalText(t, result.reason)}
        </p>
      )}
      {result?.outcome === "done" && (
        <div role="status">
          <p>{t("import.added", { count: result.added })}</p>
          {result.skipped.length > 0 && (
            <>
              <p>{t("import.skippedTitle", { count: result.skipped.length })}</p>
              <ul>
                {result.skipped.map((line) => (
                  <li key={`${line.line}-${line.text}`}>
                    {t("import.skippedLine", {
                      line: line.line,
                      text: line.text,
                      reason: t(`skip.${line.reason}`),
                    })}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}
