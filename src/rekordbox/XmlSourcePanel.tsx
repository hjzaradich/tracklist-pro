import { open } from "@tauri-apps/plugin-dialog";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { useActivityStore } from "../activity/activityStore";
import { errorMessage } from "../api/errors";
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
 */
export function XmlSourcePanel() {
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

  const startRead = (path: string | null) => {
    const marker = readMarker(source);
    read.mutate(path, { onSuccess: (job) => setStarted({ job, marker }) });
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
      {source && <ReadStatus source={source} reading={reading} />}
      {read.isError && (
        <p role="alert" className={styles.problem}>
          {errorMessage(read.error)}
        </p>
      )}
      {source?.newerExport && !busy && (
        <p className={styles.offer}>
          <span>{t("newer", { name: source.newerExport.name })}</span>
          <button
            type="button"
            className={styles.primary}
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
