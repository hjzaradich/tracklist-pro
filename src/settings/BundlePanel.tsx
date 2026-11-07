import { useQuery, useQueryClient } from "@tanstack/react-query";
import { desktopDir, join } from "@tauri-apps/api/path";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage, unwrap } from "../api/errors";
import { commands, type BundleSummary } from "../bindings";
import i18n from "../i18n";
import styles from "./settings.module.css";

const BACKUPS_QUERY_KEY = ["bundle", "backups"] as const;

/** Where the panel is: each step of making, importing or restoring a bundle (ROADMAP 4.1). */
type Step =
  | { kind: "idle" }
  | { kind: "working"; what: "creating" | "checking" }
  | { kind: "saved"; name: string }
  | { kind: "notSaved" }
  | { kind: "confirm"; from: "bundle" | "backup"; summary: BundleSummary }
  | { kind: "restarting" }
  | { kind: "failed"; error: unknown };

/** The last name in a path, whichever slash it uses. */
function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** "tracklist-pro bundle 2026-11-07.zip", from the local date. */
function suggestedName(now: Date): string {
  const two = (n: number) => String(n).padStart(2, "0");
  return `tracklist-pro bundle ${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}.zip`;
}

/** The Save dialog starts at the Desktop; if that can't be found it starts wherever the system likes. */
async function startingPath(): Promise<string> {
  const name = suggestedName(new Date());
  try {
    return await join(await desktopDir(), name);
  } catch {
    return name;
  }
}

function formatWhen(ms: number): string {
  return new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
}

/**
 * "Move to another computer" on the Settings screen (ROADMAP 4.1, barebones):
 * make a bundle and save it where the user picks, import one, restore a
 * backup. Replacing the Library always asks first, keeps a backup, and restarts.
 */
export function BundlePanel() {
  const { t } = useTranslation("settings", { keyPrefix: "bundle" });
  const titleId = useId();
  const queryClient = useQueryClient();
  const [step, setStep] = useState<Step>({ kind: "idle" });
  const backups = useQuery({
    queryKey: BACKUPS_QUERY_KEY,
    queryFn: () => unwrap(commands.listBackups()),
  });
  const busy = step.kind === "working" || step.kind === "restarting";

  /** Runs one step; whatever fails is shown in plain words. */
  const attempt = async (work: () => Promise<Step>) => {
    try {
      setStep(await work());
    } catch (error) {
      setStep({ kind: "failed", error });
    }
  };

  const create = () =>
    attempt(async () => {
      setStep({ kind: "working", what: "creating" });
      const made = await unwrap(commands.createBundle());
      const picked = await save({
        title: t("saveDialogTitle"),
        defaultPath: await startingPath(),
        filters: [{ name: t("filter"), extensions: ["zip"] }],
      });
      if (picked === null) return { kind: "notSaved" };
      await unwrap(commands.saveBundleAs(fileName(made.path), picked));
      return { kind: "saved", name: fileName(picked) };
    });

  const importBundle = () =>
    attempt(async () => {
      const picked = await open({
        title: t("openDialogTitle"),
        multiple: false,
        directory: false,
        filters: [{ name: t("filter"), extensions: ["zip"] }],
      });
      if (typeof picked !== "string") return { kind: "idle" };
      setStep({ kind: "working", what: "checking" });
      return { kind: "confirm", from: "bundle", summary: await unwrap(commands.stageImport(picked)) };
    });

  const restore = (id: string) =>
    attempt(async () => {
      setStep({ kind: "working", what: "checking" });
      return { kind: "confirm", from: "backup", summary: await unwrap(commands.stageRestore(id)) };
    });

  const confirm = () =>
    attempt(async () => {
      setStep({ kind: "restarting" });
      await unwrap(commands.confirmImport());
      void queryClient.invalidateQueries({ queryKey: BACKUPS_QUERY_KEY });
      return { kind: "restarting" };
    });

  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.panelTitle}>
        {t("title")}
      </h2>
      <p className={styles.help}>{t("intro")}</p>
      <div className={styles.actions}>
        <button type="button" className={styles.button} disabled={busy} onClick={() => void create()}>
          {t("create")}
        </button>
        <button type="button" className={styles.button} disabled={busy} onClick={() => void importBundle()}>
          {t("import")}
        </button>
      </div>
      <Status step={step} />
      {step.kind === "confirm" && (
        <Confirm
          from={step.from}
          summary={step.summary}
          onConfirm={() => void confirm()}
          onCancel={() => setStep({ kind: "idle" })}
        />
      )}
      <h3 className={styles.panelTitle}>{t("backups.title")}</h3>
      {backups.data && backups.data.length === 0 && <p className={styles.help}>{t("backups.empty")}</p>}
      <ul>
        {backups.data?.map((backup) => (
          <li key={backup.id} className={styles.row}>
            <span>
              {backup.counts
                ? t("backups.row", {
                    date: formatWhen(backup.createdMs),
                    tracks: backup.counts.tracks,
                    crates: backup.counts.crates,
                  })
                : t("backups.rowUnknown", { date: formatWhen(backup.createdMs) })}
            </span>
            <button type="button" className={styles.button} disabled={busy} onClick={() => void restore(backup.id)}>
              {t("backups.restore")}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Status({ step }: { step: Step }) {
  const { t } = useTranslation("settings", { keyPrefix: "bundle" });
  if (step.kind === "failed") {
    return (
      <p role="alert" className={styles.error}>
        {errorMessage(step.error)}
      </p>
    );
  }
  const text =
    step.kind === "working"
      ? t(step.what)
      : step.kind === "saved"
        ? t("saved", { name: step.name })
        : step.kind === "notSaved"
          ? t("notSaved")
          : step.kind === "restarting"
            ? t("restarting")
            : null;
  return text === null ? null : (
    <p role="status" className={styles.help}>
      {text}
    </p>
  );
}

function Confirm({
  from,
  summary,
  onConfirm,
  onCancel,
}: {
  from: "bundle" | "backup";
  summary: BundleSummary;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("settings", { keyPrefix: "bundle.confirm" });
  const titleId = useId();
  const detailId = useId();
  const { counts } = summary;
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={detailId}
      className={styles.confirm}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <p id={titleId} className={styles.confirmTitle}>
        {t(from === "bundle" ? "importTitle" : "restoreTitle")}
      </p>
      <div id={detailId} className={styles.confirmDetail}>
        <p>
          {summary.appVersion === null
            ? t("kept", { date: formatWhen(summary.createdMs) })
            : t("made", { date: formatWhen(summary.createdMs), version: summary.appVersion })}
        </p>
        <ul>
          <li>{t("tracks", { value: counts.tracks })}</li>
          <li>{t("crates", { value: counts.crates })}</li>
          <li>{t("playlists", { value: counts.playlists })}</li>
          <li>{t("tags", { value: counts.tags })}</li>
          <li>{t("folders", { value: counts.musicFolders })}</li>
        </ul>
        <p>{t("detail")}</p>
        {from === "bundle" && <p>{t("machine")}</p>}
        <p>{t("drives")}</p>
      </div>
      <div className={styles.actions}>
        <button type="button" className={styles.primary} onClick={onConfirm}>
          {t("replace")}
        </button>
        <button type="button" className={styles.button} onClick={onCancel}>
          {t("cancel")}
        </button>
      </div>
    </div>
  );
}
