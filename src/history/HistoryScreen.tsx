import { Link } from "@tanstack/react-router";
import { open } from "@tauri-apps/plugin-dialog";
import type { TFunction } from "i18next";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { HistoryOutcome, Play, SessionSummary, WishlistOutcome } from "../bindings";
import stage from "../shell/StageScreen.module.css";
import { useWishlists } from "../wishlist/useWishlist";
import { refusalText as wishlistRefusalText } from "../wishlist/wishlistText";
import { GigStickImport } from "./GigStickImport";
import { useAddToWishlist, useHistoryAction, usePlays, useSessions } from "./useHistory";
import styles from "./history.module.css";

/**
 * Play history, at `/history` (ROADMAP 3.6): import an m3u8 file exported
 * from rekordbox's History as one set, or a gig stick's history lists, see the sets, and a set's plays in
 * order, matched to your music or not. A set's date can be corrected, and
 * while wishlists are on, the plays you don't have can go to a wishlist.
 * Every change is one step for the top bar's Undo.
 */
export function HistoryScreen() {
  const { t } = useTranslation("history");
  const sessions = useSessions();
  const [chosen, setChosen] = useState<number | null>(null);
  const action = useHistoryAction();
  const [result, setResult] = useState<HistoryOutcome | null>(null);

  const finish = (outcome: HistoryOutcome) => {
    setResult(outcome);
    if (outcome.outcome === "done") setChosen(outcome.sessionId);
    else if (outcome.sessionId !== null) setChosen(outcome.sessionId);
  };
  const chooseFile = async () => {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: t("import.filter"), extensions: ["m3u8", "m3u"] }],
    });
    if (typeof picked !== "string") return;
    action.mutate({ kind: "import", path: picked }, { onSuccess: finish });
  };

  const list = sessions.data ?? [];
  const session = list.find((s) => s.id === chosen) ?? null;
  const failure = sessions.error ?? action.error;

  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
      </div>
      <div className={stage.body}>
        <div className={styles.screen}>
          <div className={styles.row}>
            <button
              type="button"
              className={styles.primary}
              disabled={action.isPending}
              onClick={() => void chooseFile()}
            >
              {t("import.choose")}
            </button>
          </div>
          <p className={styles.muted}>{t("import.hint")}</p>
          <GigStickImport onImported={setChosen} />
          {failure && (
            <p role="alert" className={styles.problem}>
              {errorMessage(failure)}
            </p>
          )}
          {result !== null && <Result outcome={result} />}
          <nav aria-label={t("sessions.label")}>
            {sessions.data !== undefined && list.length === 0 ? (
              <p className={styles.muted}>{t("sessions.none")}</p>
            ) : (
              <ul className={styles.lists}>
                {list.map((s) => (
                  <li key={s.id}>
                    <button
                      type="button"
                      className={styles.listButton}
                      aria-pressed={s.id === chosen}
                      onClick={() => setChosen(s.id)}
                    >
                      <span>{s.name}</span>
                      <span className={styles.muted}>
                        {t("sessions.summary", { date: s.playedOn, count: s.plays, matched: s.matched })}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </nav>
          {session === null ? (
            <p className={styles.muted}>{t("plays.choose")}</p>
          ) : (
            <SessionPlays key={session.id} session={session} />
          )}
        </div>
      </div>
    </div>
  );
}

/** What the last import or date change came to. */
function Result({ outcome }: { outcome: HistoryOutcome }) {
  const { t } = useTranslation("history");
  if (outcome.outcome === "refused") {
    return (
      <p role="alert" className={styles.problem}>
        {t(`refusal.${outcome.reason}`)}
      </p>
    );
  }
  return (
    <div role="status">
      {outcome.operationId !== null && (
        <p>{t("import.done", { count: outcome.plays, matched: outcome.matched })}</p>
      )}
      {outcome.skippedLines > 0 && (
        <p className={styles.muted}>{t("import.skippedLines", { count: outcome.skippedLines })}</p>
      )}
    </div>
  );
}

/** A play as the file named it: artist and title, or the file's name. */
function playName(t: TFunction<"history">, play: Play): string {
  if (play.artist !== null && play.title !== null) {
    return t("plays.track", { artist: play.artist, title: play.title });
  }
  return play.title ?? play.path.split(/[/\\]/).pop() ?? play.path;
}

/** The chosen set: its date, its plays in order, and the wishlist offer. */
function SessionPlays({ session }: { session: SessionSummary }) {
  const { t } = useTranslation("history");
  const plays = usePlays(session.id);
  const action = useHistoryAction();
  const [date, setDate] = useState(session.playedOn);
  const [refused, setRefused] = useState<HistoryOutcome | null>(null);
  const rows = plays.data?.plays ?? [];
  const unmatched = rows.filter((p) => p.recordingId === null).length;

  return (
    <section aria-label={t("plays.label")} className={styles.screen}>
      <h2 className={styles.sessionName}>{session.name}</h2>
      <div className={styles.row}>
        <label className={styles.row}>
          {t("plays.date")}
          <input type="date" value={date} onChange={(event) => setDate(event.target.value)} />
        </label>
        <button
          type="button"
          className={styles.action}
          disabled={action.isPending || date === session.playedOn}
          onClick={() =>
            action.mutate(
              { kind: "setDate", sessionId: session.id, playedOn: date },
              { onSuccess: (outcome) => setRefused(outcome.outcome === "refused" ? outcome : null) },
            )
          }
        >
          {t("plays.saveDate")}
        </button>
      </div>
      {refused !== null && <Result outcome={refused} />}
      {(plays.error ?? action.error) && (
        <p role="alert" className={styles.problem}>
          {errorMessage(plays.error ?? action.error)}
        </p>
      )}
      <ol className={styles.plays}>
        {rows.map((play) => (
          <li key={play.id}>
            <span>{playName(t, play)}</span>
            {play.recordingId === null && <span className={styles.notOwned}>{t("plays.notOwned")}</span>}
          </li>
        ))}
      </ol>
      {plays.data?.wishlistOffer === true && unmatched > 0 && <WishlistOffer sessionId={session.id} />}
    </section>
  );
}

/** Adds the set's plays you don't have to a wishlist; shown only while wishlists are on. */
function WishlistOffer({ sessionId }: { sessionId: number }) {
  const { t } = useTranslation("history");
  const { t: tw } = useTranslation("wishlist");
  const lists = useWishlists(true);
  const add = useAddToWishlist();
  const [picked, setPicked] = useState<number | null>(null);
  const [result, setResult] = useState<WishlistOutcome | null>(null);
  const summaries = lists.data?.outcome === "lists" ? lists.data.lists : [];
  const target = picked ?? summaries[0]?.id ?? null;

  if (lists.data !== undefined && summaries.length === 0) {
    return (
      <div className={styles.row}>
        <p className={styles.muted}>{t("wishlist.noLists")}</p>
        <Link to="/wishlists">{t("wishlist.openWishlists")}</Link>
      </div>
    );
  }
  return (
    <div>
      <div className={styles.row}>
        <label className={styles.row}>
          {t("wishlist.label")}
          <select value={target ?? ""} onChange={(event) => setPicked(Number(event.target.value))}>
            {summaries.map((list) => (
              <option key={list.id} value={list.id}>
                {list.name}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className={styles.action}
          disabled={add.isPending || target === null}
          onClick={() => {
            if (target !== null) add.mutate({ sessionId, wishlistId: target }, { onSuccess: setResult });
          }}
        >
          {t("wishlist.add")}
        </button>
      </div>
      {add.error && (
        <p role="alert" className={styles.problem}>
          {errorMessage(add.error)}
        </p>
      )}
      {result?.outcome === "refused" && (
        <p role="alert" className={styles.problem}>
          {wishlistRefusalText(tw, result.reason)}
        </p>
      )}
      {result?.outcome === "done" && (
        <div role="status">
          <p>{t("wishlist.added", { count: result.added })}</p>
          {result.skippedTotal > 0 && (
            <p className={styles.muted}>{t("wishlist.skipped", { count: result.skippedTotal })}</p>
          )}
        </div>
      )}
    </div>
  );
}
