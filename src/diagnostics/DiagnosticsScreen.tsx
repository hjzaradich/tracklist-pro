import { Link } from "@tanstack/react-router";
import type { TFunction } from "i18next";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LogEvent, EventKind } from "../bindings";
import settings from "../settings/settings.module.css";
import stage from "../shell/StageScreen.module.css";
import styles from "./diagnostics.module.css";
import { useEventLog, usePreviewReport, useSaveReport } from "./useDiagnostics";

/** Every kind of event, in the order the filter lists them. */
const KINDS: readonly EventKind[] = [
  "jobStarted",
  "jobFinished",
  "jobFailed",
  "jobCancelled",
  "rekordboxReadOk",
  "rekordboxReadFailed",
  "databaseReadOk",
  "databaseReadFailed",
  "sendPrepared",
  "sendDone",
  "sendRefused",
  "startup",
  "startupAsked",
];

/**
 * Diagnostics, at `/diagnostics` (ROADMAP 4.3): the event log, and a report
 * for a bug report. A screen reached from Settings. Nothing is sent, uploaded
 * or opened: the report is shown, saved into the app's data folder, and
 * copied only when the user presses Copy.
 */
export function DiagnosticsScreen() {
  const { t } = useTranslation("diagnostics");
  const logId = useId();
  const reportId = useId();
  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
      </div>
      <div className={stage.body}>
        <div className={settings.column}>
          <Link to="/settings" className={styles.back}>
            {t("back")}
          </Link>
          <section className={settings.panel} aria-labelledby={logId}>
            <h2 id={logId} className={settings.panelTitle}>
              {t("log.title")}
            </h2>
            <EventLog />
          </section>
          <section className={settings.panel} aria-labelledby={reportId}>
            <h2 id={reportId} className={settings.panelTitle}>
              {t("report.title")}
            </h2>
            <Report />
          </section>
        </div>
      </div>
    </div>
  );
}

function EventLog() {
  const { t, i18n } = useTranslation("diagnostics");
  const { t: ta } = useTranslation("activity");
  const filterId = useId();
  const [filter, setFilter] = useState<EventKind | "all">("all");
  const log = useEventLog();
  if (log.isError) return <p className={settings.error}>{t("log.error")}</p>;
  if (!log.data) return null;
  const when = new Intl.DateTimeFormat(i18n.language, { dateStyle: "short", timeStyle: "medium" });
  const events = log.data.events.filter((event) => filter === "all" || event.kind === filter);
  return (
    <>
      <p className={settings.help}>{t("log.help")}</p>
      {log.data.lostWrites > 0 && (
        <p role="status" className={settings.error}>
          {t("log.lost", { count: log.data.lostWrites })}
        </p>
      )}
      <div className={styles.filter}>
        <label htmlFor={filterId}>{t("log.filter")}</label>
        <select
          id={filterId}
          className={settings.input}
          value={filter}
          onChange={(event) => setFilter(event.target.value as EventKind | "all")}
        >
          <option value="all">{t("log.all")}</option>
          {KINDS.map((kind) => (
            <option key={kind} value={kind}>
              {t(`kind.${kind}`)}
            </option>
          ))}
        </select>
      </div>
      {events.length === 0 ? (
        <p className={settings.status}>{t(log.data.events.length === 0 ? "log.empty" : "log.noMatch")}</p>
      ) : (
        <div className={styles.list}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th scope="col">{t("log.time")}</th>
                <th scope="col">{t("log.what")}</th>
                <th scope="col">{t("log.task")}</th>
                <th scope="col">{t("log.reason")}</th>
                <th scope="col">{t("log.detail")}</th>
              </tr>
            </thead>
            <tbody>
              {events.map((event, index) => (
                <tr key={`${event.atMs}-${index}`}>
                  <td>{when.format(new Date(event.atMs))}</td>
                  <td>{t(`kind.${event.kind}`)}</td>
                  <td>{event.job ? ta(`task.${event.job}`) : ""}</td>
                  <td>{event.reason ? t(`reason.${event.reason}`) : ""}</td>
                  <td>{details(event, t)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function details(event: LogEvent, t: TFunction<"diagnostics">): string {
  const parts: string[] = [];
  if (event.ms !== null) parts.push(t("log.took", { ms: event.ms }));
  if (event.count !== null) parts.push(t("log.count", { count: event.count }));
  return parts.join(", ");
}

function Report() {
  const { t } = useTranslation("diagnostics");
  const previewId = useId();
  const pathId = useId();
  const preview = usePreviewReport();
  const save = useSaveReport();
  const [copied, setCopied] = useState<"report" | "path" | "failed" | null>(null);
  const text = preview.data;
  const saved = save.data;

  const copy = async (what: "report" | "path", value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(what);
    } catch {
      setCopied("failed");
    }
  };
  const create = () => {
    setCopied(null);
    save.reset();
    preview.mutate();
  };

  return (
    <>
      <p className={settings.help}>{t("report.help")}</p>
      <div className={settings.actions}>
        <button type="button" className={settings.button} disabled={preview.isPending} onClick={create}>
          {t("report.create")}
        </button>
      </div>
      {preview.isPending && (
        <p role="status" className={settings.status}>
          {t("report.creating")}
        </p>
      )}
      {preview.isError && (
        <p role="alert" className={settings.error}>
          {t("report.createError")}
        </p>
      )}
      {text !== undefined && (
        <>
          <label htmlFor={previewId}>{t("report.previewLabel")}</label>
          <textarea id={previewId} className={styles.preview} readOnly value={text} />
          <div className={settings.actions}>
            {saved === undefined && (
              <button
                type="button"
                className={settings.button}
                disabled={save.isPending}
                onClick={() => save.mutate()}
              >
                {save.isPending ? t("report.saving") : t("report.save")}
              </button>
            )}
            <button type="button" className={settings.button} onClick={() => void copy("report", text)}>
              {t("report.copy")}
            </button>
          </div>
          {save.isError && (
            <p role="alert" className={settings.error}>
              {t("report.saveError")}
            </p>
          )}
        </>
      )}
      {saved !== undefined && (
        <>
          <p role="status" className={settings.status}>
            {t("report.saved")}
          </p>
          <label htmlFor={pathId}>{t("report.pathLabel")}</label>
          <div className={styles.pathRow}>
            <input
              id={pathId}
              className={`${settings.input} ${styles.path}`}
              readOnly
              value={saved.path}
              onFocus={(event) => event.target.select()}
            />
            <button type="button" className={settings.button} onClick={() => void copy("path", saved.path)}>
              {t("report.copyPath")}
            </button>
          </div>
        </>
      )}
      {copied !== null && (
        <p role="status" className={copied === "failed" ? settings.error : settings.status}>
          {copied === "failed" ? t("report.copyFailed") : t("report.copied")}
        </p>
      )}
    </>
  );
}
