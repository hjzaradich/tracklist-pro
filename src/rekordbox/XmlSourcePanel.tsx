import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useActivityStore } from "../activity/activityStore";
import { CommandError, errorMessage } from "../api/errors";
import type { XmlSource } from "../bindings";
import styles from "./XmlSourcePanel.module.css";
import { readMarker, useReadXml, useSetWatch, useXmlSource } from "./useXmlSource";

/** A read under way: its job, and the source's read marker when it began. */
type Reading = { job: number; marker: string };

/**
 * Where the rekordbox collection comes from (1aB-11): the XML export the
 * user saves from rekordbox and picks here, read into the snapshot as a job.
 * With the watch on, a newer export (the chosen file saved again, or one in
 * Documents) is offered, never read unasked.
 *
 * The offer's button is the screen's one filled button, unless `quiet`: once
 * the Library has tracks, Send is the Overview's (1cB-11).
 */
export function XmlSourcePanel({ quiet = false }: { quiet?: boolean }) {
  const { t } = useTranslation("rekordbox");
  const titleId = useId();
  const [started, setStarted] = useState<Reading | null>(null);
  // Activity has seen the read's job and no longer holds it: it ended
  // (cancelled, say) without the source recording anything.
  const jobEnded = useActivityStore(
    (state) =>
      started !== null && state.seen.has(started.job) && state.jobs[started.job] === undefined,
  );
  const { data: source } = useXmlSource(started !== null && !jobEnded ? started.marker : null);
  // A read is under way until the source records it, done or failed.
  const reading = started !== null && !jobEnded && readMarker(source) === started.marker;
  const read = useReadXml();
  const setWatch = useSetWatch();

  // An export older than the newest database read, waiting for the user's word.
  const [older, setOlder] = useState<OlderExport | null>(null);

  const startRead = (path: string | null, allowOlder = false) => {
    const marker = readMarker(source);
    setOlder(null);
    read.mutate(
      { path, allowOlder },
      {
        onSuccess: (job) => setStarted({ job, marker }),
        onError: (error) => {
          const ipc = error instanceof CommandError ? error.error : null;
          if (ipc?.kind === "xmlOlderThanDatabaseRead") {
            setOlder({
              path,
              xmlAt: String(ipc.params.xmlAt ?? ""),
              readAt: String(ipc.params.readAt ?? ""),
            });
          }
        },
      },
    );
  };

  const choose = async () => {
    const picked = await open({
      multiple: false,
      directory: false,
      defaultPath: source?.exportFolder ?? undefined,
      filters: [{ name: t("fileFilter"), extensions: ["xml"] }],
    });
    if (typeof picked === "string") startRead(picked);
  };

  const busy = reading || read.isPending;

  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.title}>
        {t("title")}
      </h2>
      {source?.path ? (
        <p className={styles.path} title={source.path}>
          {source.path}
        </p>
      ) : (
        <>
          <p className={styles.muted}>{t("noneChosen")}</p>
          <p className={styles.muted}>{t("howTo")}</p>
        </>
      )}
      <p className={styles.muted}>{t("myTagTip")}</p>
      {source && <ReadStatus source={source} reading={reading} />}
      {older && (
        <OlderConfirm
          older={older}
          onReadAnyway={() => startRead(older.path, true)}
          onCancel={() => {
            setOlder(null);
            // The held-back read is not a failure to show.
            read.reset();
          }}
        />
      )}
      {read.isError && !older && (
        <p role="alert" className={styles.problem}>
          {errorMessage(read.error)}
        </p>
      )}
      {source?.newerExport && !busy && (
        <p className={styles.offer}>
          <span>{t("newer", { name: source.newerExport.name })}</span>
          <button
            type="button"
            className={quiet ? styles.button : styles.primary}
            onClick={() => startRead(source.newerExport?.path ?? null)}
          >
            {t("readNewer")}
          </button>
        </p>
      )}
      <div className={styles.actions}>
        <button type="button" className={styles.button} disabled={busy} onClick={() => void choose()}>
          {source?.path ? t("chooseAnother") : t("choose")}
        </button>
        {source?.path && (
          <button
            type="button"
            className={styles.button}
            disabled={busy}
            onClick={() => startRead(null)}
          >
            {t("readAgain")}
          </button>
        )}
      </div>
      <label className={styles.watch}>
        <input
          type="checkbox"
          checked={source?.watch ?? false}
          disabled={source === undefined || setWatch.isPending}
          onChange={(event) => setWatch.mutate(event.target.checked)}
        />
        {t("watch")}
      </label>
    </section>
  );
}

/** An export the backend held back, and the two times it compared. */
type OlderExport = { path: string | null; xmlAt: string; readAt: string };

/**
 * Asks before an export older than the newest read of rekordbox's database
 * replaces it. Cancel is the focused, safe choice.
 */
function OlderConfirm({
  older,
  onReadAnyway,
  onCancel,
}: {
  older: OlderExport;
  onReadAnyway: () => void;
  onCancel: () => void;
}) {
  const { t, i18n } = useTranslation("rekordbox");
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  const titleId = useId();
  // A time that can't be read is shown as it came, never thrown on.
  const when = (iso: string) => {
    const date = new Date(iso);
    return Number.isNaN(date.getTime())
      ? iso
      : new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
          date,
        );
  };
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <p id={titleId} className={styles.warning}>
        {t("older.message", { xmlAt: when(older.xmlAt), readAt: when(older.readAt) })}
      </p>
      <div className={styles.actions}>
        <button type="button" className={styles.button} onClick={onReadAnyway}>
          {t("older.readAnyway")}
        </button>
        <button ref={cancel} type="button" className={styles.button} onClick={onCancel}>
          {t("older.cancel")}
        </button>
      </div>
    </div>
  );
}

/** The last read, or the last failure, or that a read is under way. */
function ReadStatus({ source, reading }: { source: XmlSource; reading: boolean }) {
  const { t, i18n } = useTranslation("rekordbox");
  if (reading) return <p role="status">{t("reading")}</p>;
  const { lastRead, lastFailure } = source;
  const when = (iso: string) =>
    new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
      new Date(iso),
    );
  return (
    <>
      {lastRead ? (
        <p role="status">
          {t("lastRead", { when: when(lastRead.readAt), count: lastRead.summary.tracks })}
        </p>
      ) : (
        !lastFailure && <p className={styles.muted}>{t("neverRead")}</p>
      )}
      {lastRead && !lastRead.summary.complete && (
        <p className={styles.warning}>{t("incomplete")}</p>
      )}
      {lastRead && lastRead.summary.notStored > 0 && (
        <p className={styles.warning}>{t("notStored", { count: lastRead.summary.notStored })}</p>
      )}
      {lastFailure && (
        <p role="alert" className={styles.problem}>
          {t(`failed.${lastFailure.reason}`, { path: lastFailure.path })}
        </p>
      )}
    </>
  );
}
