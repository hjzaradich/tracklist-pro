import { useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { useActivityStore } from "../activity/activityStore";
import { errorMessage } from "../api/errors";
import type { Preflight, SendState, TrackLabel } from "../bindings";
import { useXmlSource, XML_SOURCE_QUERY_KEY } from "../rekordbox/useXmlSource";
import styles from "./SendChecklist.module.css";
import { SEND_STATE_QUERY_KEY, usePrepareSend, useSendState, useWriteSend } from "./useSend";

/** A step under way: its job, and the state's revision when it began. */
type Waiting = { step: "prepare" | "write"; job: number; revision: number };

/** At most this many rows of a list are shown; the rest are counted. */
export const MAX_ROWS = 50;

/**
 * The guided send (1aF-1, ROADMAP 1.9): export from rekordbox, review what
 * will be sent, write the file, import it in rekordbox. Every step is the
 * user's; nothing is written before "Write the file".
 *
 * `afterSend` is shown in the last step: the lists of what to tidy up in
 * rekordbox by hand (1aF-2).
 */
export function SendChecklist({
  onClose,
  afterSend,
}: {
  onClose: () => void;
  afterSend?: ReactNode;
}) {
  const { t, i18n } = useTranslation(["send", "rekordbox"]);
  const titleId = useId();
  const queryClient = useQueryClient();
  const [waiting, setWaiting] = useState<Waiting | null>(null);
  // Activity has seen the step's job and no longer holds it: it ended.
  const jobEnded = useActivityStore(
    (activity) =>
      waiting !== null && activity.seen.has(waiting.job) && activity.jobs[waiting.job] === undefined,
  );
  const { data: state } = useSendState(waiting !== null && !jobEnded ? waiting.revision : null);
  useEffect(() => {
    if (jobEnded) void queryClient.invalidateQueries({ queryKey: SEND_STATE_QUERY_KEY });
  }, [jobEnded, queryClient]);
  // A step's end may have changed what the rekordbox source says (a new
  // read, or why one failed).
  const revision = state?.revision;
  useEffect(() => {
    void queryClient.invalidateQueries({ queryKey: XML_SOURCE_QUERY_KEY });
  }, [revision, queryClient]);
  // A step is under way until the state records its end.
  const busy = waiting !== null && !jobEnded && state?.revision === waiting.revision;
  const prepare = usePrepareSend();
  const write = useWriteSend();
  const { data: source } = useXmlSource(null);
  // The confirm is for one preflight: a new one starts unticked.
  const [confirmedFor, setConfirmedFor] = useState<string | null>(null);

  const when = (date: Date) =>
    new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(date);

  const preflight = state?.preflight ?? null;
  const confirmed = preflight !== null && confirmedFor === preflight.token;
  const failure = !busy ? (state?.failure ?? null) : null;
  // The step the failure belongs under: the one that ended last.
  const failedStep = state?.step ?? "write";
  const working = busy || prepare.isPending || write.isPending;

  const startPrepare = (path: string | null) => {
    const since = state?.revision ?? 0;
    write.reset();
    prepare.mutate(path, {
      onSuccess: (job) => setWaiting({ step: "prepare", job, revision: since }),
    });
  };

  const choose = async () => {
    const picked = await open({
      multiple: false,
      directory: false,
      defaultPath: source?.exportFolder ?? undefined,
      filters: [{ name: t("export.fileFilter"), extensions: ["xml"] }],
    });
    if (typeof picked === "string") startPrepare(picked);
  };

  const startWrite = () => {
    if (!preflight || !state) return;
    const since = state.revision;
    write.mutate(
      { token: preflight.token, confirmed },
      { onSuccess: (job) => setWaiting({ step: "write", job, revision: since }) },
    );
  };

  const canGo =
    preflight !== null && preflight.canSend && (!preflight.needsConfirm || confirmed) && !working;
  const dialogs = state?.sent?.knownTracks ?? preflight?.knownTracks ?? null;

  return (
    <section className={styles.checklist} aria-labelledby={titleId}>
      <div className={styles.head}>
        <h2 id={titleId} className={styles.title}>
          {t("title")}
        </h2>
        <button type="button" className={styles.button} onClick={onClose}>
          {t("close")}
        </button>
      </div>
      <p className={styles.warning}>{t("warning")}</p>
      <ol className={styles.steps}>
        <li>
          <h3 className={styles.stepTitle}>{t("export.title")}</h3>
          <p className={styles.muted}>{t("export.howTo")}</p>
          {state?.exportPath ? (
            <p className={styles.path} title={state.exportPath}>
              {state.exportPath}
            </p>
          ) : (
            state && <p className={styles.muted}>{t("export.noneChosen")}</p>
          )}
          {busy && waiting?.step === "prepare" && <p role="status">{t("export.reading")}</p>}
          {preflight && (
            <p role="status">
              {t("export.saved", { when: when(new Date(preflight.export.modifiedMs)) })}
            </p>
          )}
          {failure && failedStep === "prepare" && (
            <p role="alert" className={styles.problem}>
              {failure === "readFailed" && source?.lastFailure
                ? t(`rekordbox:failed.${source.lastFailure.reason}`, {
                    path: source.lastFailure.path,
                  })
                : t(`failure.${failure}`)}
            </p>
          )}
          {prepare.isError && (
            <p role="alert" className={styles.problem}>
              {errorMessage(prepare.error)}
            </p>
          )}
          <div className={styles.actions}>
            {state?.exportPath && (
              <button
                type="button"
                className={styles.primary}
                disabled={working}
                onClick={() => startPrepare(null)}
              >
                {t("export.read")}
              </button>
            )}
            <button
              type="button"
              className={styles.button}
              disabled={working}
              onClick={() => void choose()}
            >
              {state?.exportPath ? t("export.chooseAnother") : t("export.choose")}
            </button>
          </div>
        </li>
        <li>
          <h3 className={styles.stepTitle}>{t("review.title")}</h3>
          {preflight ? (
            <Review
              preflight={preflight}
              confirmed={confirmed}
              onConfirm={(ticked) => setConfirmedFor(ticked ? preflight.token : null)}
            />
          ) : (
            <p className={styles.muted}>{t("review.notYet")}</p>
          )}
        </li>
        <li>
          <h3 className={styles.stepTitle}>{t("write.title")}</h3>
          {state && <SendFile path={state.filePath} />}
          {busy && waiting?.step === "write" && <p role="status">{t("write.writing")}</p>}
          {state?.sent && !busy && (
            <p role="status">{t("write.written", { when: when(new Date(state.sent.at)) })}</p>
          )}
          {failure && failedStep === "write" && (
            <p role="alert" className={styles.problem}>
              {t(`failure.${failure}`)}
            </p>
          )}
          {write.isError && (
            <p role="alert" className={styles.problem}>
              {errorMessage(write.error)}
            </p>
          )}
          <div className={styles.actions}>
            <button type="button" className={styles.primary} disabled={!canGo} onClick={startWrite}>
              {t("write.go")}
            </button>
          </div>
        </li>
        <li>
          <h3 className={styles.stepTitle}>{t("import.title")}</h3>
          <ul className={styles.list}>
            <li>{t("import.show")}</li>
            <li>{t("import.point")}</li>
            <li>{t("import.refresh")}</li>
            <li>{t("import.tracks")}</li>
            <li>
              {dialogs === null
                ? t("import.dialogsUnknown")
                : dialogs === 0
                  ? t("import.dialogsNone")
                  : t("import.dialogs", { count: dialogs })}
            </li>
            <li>{t("import.playlists")}</li>
          </ul>
        </li>
        <li>
          <h3 className={styles.stepTitle}>{t("after.title")}</h3>
          {/* 1aF-2 mounts its after-send lists here. */}
          <div data-slot="after-send">{afterSend}</div>
        </li>
      </ol>
    </section>
  );
}

/** What a list calls a track: its title or its file's name, with its artist. */
function useTrackName() {
  const { t } = useTranslation("send");
  return (track: TrackLabel) => {
    const title = track.title ?? track.fileName ?? "";
    return track.artist ? t("trackWithArtist", { title, artist: track.artist }) : title;
  };
}

/** The first {@link MAX_ROWS} of `rows`, then how many more there are. */
function Rows<T>({ rows, children }: { rows: T[]; children: (row: T) => ReactNode }) {
  const { t } = useTranslation("send");
  return (
    <ul className={styles.list}>
      {rows.slice(0, MAX_ROWS).map(children)}
      {rows.length > MAX_ROWS && (
        <li className={styles.muted}>{t("more", { count: rows.length - MAX_ROWS })}</li>
      )}
    </ul>
  );
}

/** The preflight: what would be sent, what's left out, and what needs a yes. */
function Review({
  preflight,
  confirmed,
  onConfirm,
}: {
  preflight: Preflight;
  confirmed: boolean;
  onConfirm: (ticked: boolean) => void;
}) {
  const { t } = useTranslation("send");
  const name = useTrackName();
  const { refusal } = preflight;
  return (
    <>
      {refusal && (
        <p role="alert" className={styles.problem}>
          {t(`review.refused.${refusal.reason}`, { path: refusal.path.join(" / ") })}
        </p>
      )}
      {preflight.nothingToSend && !refusal && <p role="status">{t("review.nothing")}</p>}
      {!refusal && !preflight.nothingToSend && (
        <>
          <p>{t("review.new", { count: preflight.newTracks })}</p>
          <p>{t("review.known", { count: preflight.knownTracks })}</p>
        </>
      )}
      {preflight.leftOut.length > 0 && (
        <>
          <p className={styles.warning}>
            {t("review.leftOut", { count: preflight.leftOut.length })}
          </p>
          <Rows rows={preflight.leftOut}>
            {(left) => (
              <li key={left.track.libraryTrack}>
                {name(left.track)}
                <span className={styles.reason}>
                  {t(`review.leftOutReason.${left.reason}`, { attribute: left.attribute ?? "" })}
                </span>
              </li>
            )}
          </Rows>
        </>
      )}
      {preflight.otherFile.length > 0 && (
        <>
          <p className={styles.warning}>
            {t("review.otherFile", { count: preflight.otherFile.length })}
          </p>
          <Rows rows={preflight.otherFile}>
            {(other) => (
              <li key={other.track.libraryTrack}>
                {name(other.track)}
                <span className={styles.detail}>
                  {t("review.libraryFile", { path: other.libraryFile ?? "" })}
                </span>
                <span className={styles.detail}>
                  {t("review.rekordboxFile", { path: other.rekordboxFile ?? "" })}
                </span>
              </li>
            )}
          </Rows>
        </>
      )}
      {preflight.losesEntries.length > 0 && (
        <>
          <p className={styles.warning}>{t("review.losesEntries")}</p>
          <Rows rows={preflight.losesEntries}>
            {(loses) => (
              <li key={`${loses.kind}/${loses.path.join("/")}`}>
                {t("review.losesEntriesRow", {
                  name: loses.path.join(" / "),
                  count: loses.lost,
                  entries: loses.entries,
                })}
              </li>
            )}
          </Rows>
        </>
      )}
      {preflight.export.notStored > 0 && (
        <p className={styles.warning}>
          {t("review.notStored", { count: preflight.export.notStored })}
        </p>
      )}
      {preflight.needsConfirm && preflight.canSend && (
        <label className={styles.confirm}>
          <input
            type="checkbox"
            checked={confirmed}
            onChange={(event) => onConfirm(event.target.checked)}
          />
          {t("review.confirm")}
        </label>
      )}
    </>
  );
}

/** The send file's path, to select or copy into rekordbox's file dialog. */
function SendFile({ path }: { path: SendState["filePath"] }) {
  const { t } = useTranslation("send");
  const pathId = useId();
  const [copied, setCopied] = useState<boolean | null>(null);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(path);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };
  return (
    <>
      <div className={styles.file}>
        <label htmlFor={pathId} className={styles.muted}>
          {t("write.path")}
        </label>
        <input
          id={pathId}
          className={styles.filePath}
          readOnly
          value={path}
          onFocus={(event) => event.target.select()}
        />
        <button type="button" className={styles.button} onClick={() => void copy()}>
          {t("write.copy")}
        </button>
      </div>
      {copied !== null && (
        <p role="status" className={copied ? styles.muted : styles.problem}>
          {copied ? t("write.copied") : t("write.copyFailed")}
        </p>
      )}
    </>
  );
}
