import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { StickImport, StickList } from "../bindings";
import { useStickImport, useStickLists } from "./useHistory";
import styles from "./history.module.css";

/** Today as `YYYY-MM-DD`, in the user's time zone: the default "played on" day. */
export function localToday(now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/**
 * Imports play history from a gig stick (ROADMAP 3.6): pick the stick's top
 * folder, see its history lists, tick the ones to import, give the day they
 * were played, import. Each list becomes one set; the whole import is one
 * step for Undo. The stick is only read.
 */
export function GigStickImport({ onImported }: { onImported: (sessionId: number) => void }) {
  const { t } = useTranslation("history");
  const read = useStickLists();
  const run = useStickImport();
  const [root, setRoot] = useState<string | null>(null);
  const [ticked, setTicked] = useState<number[]>([]);
  const [date, setDate] = useState(localToday);
  const [result, setResult] = useState<StickImport | null>(null);

  const chooseStick = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (typeof picked !== "string") return;
    setRoot(picked);
    setTicked([]);
    setResult(null);
    run.reset();
    read.mutate(picked);
  };
  const listing = read.data;
  const lists: StickList[] = listing?.outcome === "listed" ? listing.lists : [];
  const toggle = (id: number) => setTicked((now) => (now.includes(id) ? now.filter((x) => x !== id) : [...now, id]));
  const importChosen = () => {
    if (root === null) return;
    run.mutate(
      { root, listIds: ticked, playedOn: date },
      {
        onSuccess: (outcome) => {
          setResult(outcome);
          const first = outcome.outcome === "done" ? outcome.sessions[0] : undefined;
          if (first !== undefined) onImported(first);
        },
      },
    );
  };
  const failure = read.error ?? run.error;

  return (
    <section aria-label={t("stick.label")} className={styles.screen}>
      <div className={styles.row}>
        <button type="button" className={styles.primary} disabled={read.isPending || run.isPending} onClick={() => void chooseStick()}>
          {t("stick.choose")}
        </button>
      </div>
      <p className={styles.muted}>{t("stick.hint")}</p>
      {failure && (
        <p role="alert" className={styles.problem}>
          {errorMessage(failure)}
        </p>
      )}
      {listing?.outcome === "refused" && (
        <p role="alert" className={styles.problem}>
          {t(`stick.refusal.${listing.reason}`)}
        </p>
      )}
      {listing?.outcome === "listed" && (
        <div className={styles.screen}>
          {lists.length === 0 ? (
            <p className={styles.muted}>{t("stick.noLists")}</p>
          ) : (
            <ul className={styles.lists}>
              {lists.map((list) => (
                <li key={list.id}>
                  <label className={styles.row}>
                    <input
                      type="checkbox"
                      checked={ticked.includes(list.id)}
                      disabled={list.plays === 0}
                      onChange={() => toggle(list.id)}
                    />
                    {t("stick.list", { name: list.name, count: list.plays })}
                  </label>
                </li>
              ))}
            </ul>
          )}
          {listing.skipped > 0 && <p className={styles.muted}>{t("stick.skipped", { count: listing.skipped })}</p>}
          <div className={styles.row}>
            <label className={styles.row}>
              {t("stick.date")}
              <input type="date" value={date} onChange={(event) => setDate(event.target.value)} />
            </label>
            <button
              type="button"
              className={styles.action}
              disabled={run.isPending || ticked.length === 0}
              onClick={importChosen}
            >
              {t("stick.import")}
            </button>
          </div>
        </div>
      )}
      {result !== null && <StickResult outcome={result} />}
    </section>
  );
}

/** What a stick import came to: sets and plays imported, plays not in your music, lists already here. */
function StickResult({ outcome }: { outcome: StickImport }) {
  const { t } = useTranslation("history");
  if (outcome.outcome === "refused") {
    return (
      <p role="alert" className={styles.problem}>
        {t(`stick.refusal.${outcome.reason}`)}
      </p>
    );
  }
  const unmatched = outcome.plays - outcome.matched;
  return (
    <div role="status">
      {outcome.sessions.length > 0 && (
        <>
          <p>{t("stick.done", { count: outcome.sessions.length })}</p>
          <p>{t("import.done", { count: outcome.plays, matched: outcome.matched })}</p>
        </>
      )}
      {unmatched > 0 && <p className={styles.muted}>{t("stick.unmatched", { count: unmatched })}</p>}
      {outcome.alreadyImported > 0 && (
        <p className={styles.muted}>{t("stick.alreadyImported", { count: outcome.alreadyImported })}</p>
      )}
      {outcome.skipped > 0 && <p className={styles.muted}>{t("stick.skipped", { count: outcome.skipped })}</p>}
    </div>
  );
}
