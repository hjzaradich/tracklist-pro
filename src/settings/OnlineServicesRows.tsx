import { useId, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { OnlineService, SaveTokenOutcome } from "../bindings";
import { SwitchRow } from "./SwitchRow";
import styles from "./settings.module.css";
import {
  useOnlineServicesStatus,
  useRemoveDiscogsToken,
  useSaveDiscogsToken,
  useSetOnlineService,
} from "./useOnlineServices";

/**
 * The lookup services under Online services (ROADMAP 2.2), below the update
 * checks: one switch each, off until turned on, and one plain line saying what
 * it sends. AcoustID can't be used while this build has no application key;
 * Discogs also takes the user's own token.
 */
export function OnlineServicesRows() {
  const { t } = useTranslation("onlineServices");
  const status = useOnlineServicesStatus().data;
  const known = status !== undefined;
  const keyMissing = status?.acoustId.keyMissing === true;
  return (
    <>
      <ServiceRow
        service="musicBrainz"
        label={t("musicBrainz.label")}
        help={t("musicBrainz.help")}
        on={status?.musicBrainz === true}
        known={known}
      />
      <ServiceRow
        service="acoustId"
        label={t("acoustId.label")}
        help={t("acoustId.help")}
        notes={keyMissing ? [t("acoustId.unavailable")] : []}
        on={status?.acoustId.on === true && !keyMissing}
        known={known}
        unavailable={keyMissing}
      />
      <ServiceRow
        service="discogs"
        label={t("discogs.label")}
        help={t("discogs.help")}
        on={status?.discogs.on === true}
        known={known}
      >
        {status?.discogs.on === true && (
          <DiscogsToken tokenSaved={status.discogs.tokenSaved} storeAvailable={status.discogs.storeAvailable} />
        )}
      </ServiceRow>
      <ServiceRow
        service="coverArt"
        label={t("coverArt.label")}
        help={t("coverArt.help")}
        on={status?.coverArt === true}
        known={known}
      />
    </>
  );
}

function ServiceRow({
  service,
  label,
  help,
  notes,
  on,
  known,
  unavailable = false,
  children,
}: {
  service: OnlineService;
  label: string;
  help: string;
  notes?: string[];
  on: boolean;
  known: boolean;
  /** The service can't be used in this build; the switch stays off and can't be changed. */
  unavailable?: boolean;
  children?: ReactNode;
}) {
  const { t } = useTranslation("onlineServices");
  const save = useSetOnlineService(service);
  const checked = save.isPending ? save.variables === true : on;
  return (
    <SwitchRow
      label={label}
      help={known ? help : undefined}
      notes={known ? notes : undefined}
      checked={checked}
      pending={!known || unavailable || save.isPending}
      error={save.isError ? t("error") : null}
      onChange={(next) => save.mutate(next)}
    >
      {children}
    </SwitchRow>
  );
}

/** What came of the last Save or Remove: the backend's outcome, or the app's own failure. */
type Notice = SaveTokenOutcome | "removed" | "failed";

/**
 * Discogs' token: typed into a password field that is never filled in and is
 * emptied on Save (the token is checked online, kept in the computer's
 * password store, and never shown again). The page where a token is made is
 * shown as text with a Copy button; the app opens no links.
 */
function DiscogsToken({ tokenSaved, storeAvailable }: { tokenSaved: boolean; storeAvailable: boolean }) {
  const { t } = useTranslation("onlineServices");
  const fieldId = useId();
  const [token, setToken] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [copied, setCopied] = useState<"copied" | "failed" | null>(null);
  const save = useSaveDiscogsToken();
  const remove = useRemoveDiscogsToken();
  const busy = save.isPending || remove.isPending;
  const address = t("discogs.tokenUrl");

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const typed = token;
    setToken("");
    setNotice(null);
    save.mutate(typed, {
      onSuccess: (outcome) => setNotice(outcome),
      onError: () => setNotice("failed"),
      // Forget the typed token (the mutation's variables) once the save is over.
      onSettled: () => save.reset(),
    });
  };
  const forget = () => {
    setNotice(null);
    remove.mutate(undefined, {
      onSuccess: () => setNotice("removed"),
      onError: () => setNotice("failed"),
    });
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(address);
      setCopied("copied");
    } catch {
      setCopied("failed");
    }
  };
  const line = save.isPending ? t("discogs.saving") : notice === null ? null : noticeText(notice);

  return (
    <div className={styles.reader}>
      <form className={styles.field} onSubmit={submit}>
        <label htmlFor={fieldId}>{t("discogs.tokenLabel")}</label>
        <input
          id={fieldId}
          type="password"
          className={styles.input}
          autoComplete="off"
          spellCheck={false}
          value={token}
          disabled={busy}
          onChange={(event) => setToken(event.target.value)}
        />
        <div className={styles.actions}>
          <button type="submit" className={styles.primary} disabled={busy || !storeAvailable}>
            {t("discogs.save")}
          </button>
          <button type="button" className={styles.button} disabled={busy || !tokenSaved} onClick={forget}>
            {t("discogs.remove")}
          </button>
        </div>
      </form>
      <p className={styles.status}>{tokenSaved ? t("discogs.saved") : t("discogs.notSaved")}</p>
      {!storeAvailable && <p className={styles.status}>{t("discogs.outcome.storeUnavailable")}</p>}
      <p role="status" className={styles.status}>
        {line}
      </p>
      <p className={styles.status}>
        {t("discogs.getLabel")} <code>{address}</code>{" "}
        <button type="button" className={styles.button} onClick={() => void copy()}>
          {t("discogs.copy")}
        </button>
      </p>
      {copied !== null && (
        <p role="status" className={styles.status}>
          {t(copied === "copied" ? "discogs.copied" : "discogs.copyFailed")}
        </p>
      )}
    </div>
  );

  function noticeText(value: Notice): string {
    switch (value) {
      case "removed":
        return t("discogs.removed");
      case "failed":
        return t("discogs.outcome.failed");
      case "saved":
        return t("discogs.outcome.saved");
      case "switchOff":
        return t("discogs.outcome.switchOff");
      case "empty":
        return t("discogs.outcome.empty");
      case "storeUnavailable":
        return t("discogs.outcome.storeUnavailable");
      case "rejected":
        return t("discogs.outcome.rejected");
      case "couldNotCheck":
        return t("discogs.outcome.couldNotCheck");
      case "storeFailed":
        return t("discogs.outcome.storeFailed");
    }
  }
}
