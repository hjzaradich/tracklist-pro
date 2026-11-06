import { open } from "@tauri-apps/plugin-dialog";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DbFailure } from "../bindings";
import { SwitchRow } from "./SwitchRow";
import styles from "./settings.module.css";
import {
  databaseReadMarker,
  useDatabaseFeature,
  useDatabaseState,
  useReadDatabase,
  useSetDatabaseFeature,
  useSetDatabaseKey,
} from "./useRekordboxDatabase";

/**
 * "Read rekordbox's library database" (2C-R1, ROADMAP 2.7): optional, off by
 * default. While off, the row is only its switch and nothing else is asked
 * of the backend. While on: the key field (a password field, emptied once the
 * key is handed over, never filled back in), Pick database, and Read now.
 */
export function RekordboxDatabaseRow() {
  const { t } = useTranslation("rekordboxDb");
  const feature = useDatabaseFeature();
  const known = feature.data !== undefined;
  const on = feature.data === true;
  const save = useSetDatabaseFeature();
  const [readingSince, setReadingSince] = useState<string | null>(null);
  const state = useDatabaseState(on, readingSince);
  const setKey = useSetDatabaseKey();
  const read = useReadDatabase();
  const [key, setKeyText] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [refused, setRefused] = useState<DbFailure | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);
  const keyId = useId();

  const data = state.data;
  const path = picked ?? data?.lastRead?.path ?? null;
  const reading = readingSince !== null && databaseReadMarker(data) === readingSince;
  const busy = reading || setKey.isPending || read.isPending;
  const checked = save.isPending ? save.variables === true : on;

  const change = (next: boolean) => {
    setSaveFailed(false);
    setRefused(null);
    save.mutate(next, { onError: () => setSaveFailed(true) });
  };

  const pick = async () => {
    const chosen = await open({
      multiple: false,
      directory: false,
      defaultPath: data?.usualFolder ?? undefined,
      filters: [{ name: t("fileFilter"), extensions: ["db"] }],
    });
    if (typeof chosen === "string") setPicked(chosen);
  };

  const readNow = async () => {
    if (path === null) return;
    setRefused(null);
    const typed = key;
    setKeyText("");
    try {
      if (typed !== "") await setKey.send(typed);
      const since = databaseReadMarker(data);
      const start = await read.mutateAsync(path);
      if (start.kind === "refused") setRefused(start.reason);
      else setReadingSince(since);
    } catch {
      setRefused("cantRead");
    }
  };

  const failure = refused ?? (reading ? null : (data?.failure ?? null));
  let status: string | null = null;
  if (reading) status = t("reading");
  else if (failure !== null) status = t(`failure.${failure}`);
  else if (data?.lastRead) status = t("done", { count: data.lastRead.summary.tracks });

  return (
    <SwitchRow
      label={t("label")}
      help={known ? t(on ? "helpOn" : "helpOff") : undefined}
      checked={checked}
      pending={!known || save.isPending}
      error={saveFailed ? t("error") : null}
      onChange={change}
    >
      {on && (
        <div className={styles.reader}>
          <div className={styles.field}>
            <label htmlFor={keyId}>{t("keyLabel")}</label>
            <input
              id={keyId}
              type="password"
              autoComplete="off"
              spellCheck={false}
              className={styles.input}
              value={key}
              disabled={busy}
              onChange={(event) => setKeyText(event.target.value)}
            />
            {data?.keySet && <p className={styles.status}>{t("keySet")}</p>}
          </div>
          <div className={styles.actions}>
            <button type="button" className={styles.button} disabled={busy} onClick={pick}>
              {t("pick")}
            </button>
            <button
              type="button"
              className={styles.button}
              disabled={busy || path === null || (key === "" && !data?.keySet)}
              onClick={readNow}
            >
              {t("read")}
            </button>
          </div>
          {path !== null && <p className={styles.status}>{t("picked", { path })}</p>}
          {status !== null && (
            <p className={styles.status} role="status">
              {status}
            </p>
          )}
        </div>
      )}
    </SwitchRow>
  );
}
