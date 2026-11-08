import { useQueryClient } from "@tanstack/react-query";
import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";
import { errorMessage } from "../api/errors";
import type {
  IdentifyChoice,
  IdentifyFieldSuggestions,
  IdentifyLookupStatus,
  IdentifySource,
  IdentifySuggestions,
  TableRow,
} from "../bindings";
import { useRegisterActions } from "../palette/registry";
import { useSelectedTrack } from "../shell/selectedTrack";
import { useWhenUndone } from "../shell/undoneStore";
import styles from "./LookUpPanel.module.css";
import {
  IDENTIFY_QUERY_KEY,
  useIdentifySuggestions,
  useLookUpTracks,
  useLookupStatus,
  useOnlineLookupAvailable,
  useUseSuggestions,
} from "./useIdentify";

type T = ReturnType<typeof useTranslation<"identify">>["t"];

/** The reasons the app has words for; any other code is shown as it is. */
const REASONS = [
  "services_off",
  "acoustid_off",
  "musicbrainz_off",
  "key_missing",
  "no_fingerprint",
  "no_match",
  "no_details",
  "offline",
  "busy",
  "refused",
  "bad_reply",
  "failed",
] as const;
type Reason = (typeof REASONS)[number];
const isReason = (code: string): code is Reason => (REASONS as readonly string[]).includes(code);

/**
 * "Look up" in the Details panel (ROADMAP 2.2): what the app suggests for the
 * selected Library track's title, artist, version, remixer, label, year, album
 * and catalogue number, each saying where it came from, with "Use" per
 * suggestion and "Use all". Suggestions from the track's own files show with
 * every online service off; "Look up" asks the services that are on, in the
 * background. Use sets the app's own value (one Undo), which reaches rekordbox
 * on the next send; nothing is applied without it. For a selection of tracks
 * it also offers "Look up N tracks", and the command palette has the same.
 *
 * Give it a `key` per track, so one track's answer doesn't stay under the next.
 */
export function LookUpPanel({ row }: { row: TableRow }) {
  const libraryTrackId = row.libraryTrackId;
  if (libraryTrackId === null) return null;
  return <Panel libraryTrackId={libraryTrackId} />;
}

function Panel({ libraryTrackId }: { libraryTrackId: number }) {
  const { t } = useTranslation("identify");
  const queryClient = useQueryClient();
  const headingId = useId();
  const offId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const suggestions = useIdentifySuggestions(libraryTrackId);
  const status = useLookupStatus(libraryTrackId);
  const available = useOnlineLookupAvailable();
  const lookUp = useLookUpTracks();
  const use = useUseSuggestions();
  const selected = useSelectedTrack((state) => state.ids);
  const [asked, setAsked] = useState<{ selected: number; sent: number; queued: boolean } | null>(null);
  const [used, setUsed] = useState<{ count: number; changed: boolean } | null>(null);

  // A finished look-up, and an Undo of a Use, change what is suggested.
  const refresh = () => void queryClient.invalidateQueries({ queryKey: IDENTIFY_QUERY_KEY });
  useWhenJobsEnd(refresh);
  useWhenUndone(() => {
    refresh();
    setUsed(null);
  });

  const many = selected.length > 1;
  const off = available === false;
  const waiting = status.data?.state === "queued" || status.data?.state === "running";

  const ask = (ids: number[], count: number) => {
    setAsked(null);
    lookUp.mutate(ids, {
      onSuccess: (job) => setAsked({ selected: count, sent: ids.length, queued: job !== null }),
    });
  };
  const askOne = () => ask([libraryTrackId], 1);
  const askSelected = () => {
    const ids = useSelectedTrack.getState().libraryIdsOf?.(selected) ?? [];
    if (ids.length === 0) setAsked({ selected: selected.length, sent: 0, queued: false });
    else ask(ids, selected.length);
  };

  useRegisterActions("identify", [
    {
      id: "identify:lookUp",
      label: t("lookUp.palette"),
      available: available === true && selected.length > 0,
      run: selected.length > 1 ? askSelected : askOne,
    },
  ]);

  const choose = (choices: IdentifyChoice[]) => {
    setUsed(null);
    use.mutate(
      { libraryTrackId, choices },
      {
        onSuccess: (operation) => {
          setUsed({ count: choices.length, changed: operation !== null });
          heading.current?.focus();
        },
      },
    );
  };

  const shown = suggestions.data === undefined ? [] : fieldsWithSuggestions(suggestions.data);
  const errors = [suggestions.error, status.error, lookUp.error, use.error].filter((error) => error != null);
  return (
    <section aria-labelledby={headingId} className={styles.section}>
      <h3 id={headingId} ref={heading} tabIndex={-1} className={styles.heading}>
        {t("lookUp.title")}
      </h3>
      <p className={styles.quiet}>{statusLine(status.data, suggestions.data, t)}</p>
      {status.data?.laterAttempt != null && (
        <p className={styles.quiet}>
          {t("lookUp.later", {
            when: whenText(status.data.laterAttempt.when),
            reason: reasonText(status.data.laterAttempt.reason, t),
          })}
        </p>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.button}
          disabled={off || waiting || lookUp.isPending}
          aria-describedby={off ? offId : undefined}
          onClick={askOne}
        >
          {status.data == null || status.data.state === "never" ? t("lookUp.action") : t("lookUp.again")}
        </button>
        {many && (
          <button
            type="button"
            className={styles.button}
            disabled={off || lookUp.isPending}
            aria-describedby={off ? offId : undefined}
            onClick={askSelected}
          >
            {t("lookUp.selected", { count: selected.length })}
          </button>
        )}
      </div>
      {off && (
        <p id={offId} className={styles.quiet}>
          {t("lookUp.off")}
        </p>
      )}
      {asked !== null && (
        <p role="status" className={styles.quiet}>
          {askedLine(asked, t)}
        </p>
      )}
      {suggestions.isPending && <p className={styles.quiet}>{t("lookUp.loading")}</p>}
      {suggestions.data !== undefined && shown.length === 0 && <p className={styles.quiet}>{t("lookUp.nothing")}</p>}
      {shown.length > 0 && (
        <>
          <dl className={styles.fields}>
            {shown.map((field) => (
              <FieldRow
                key={field.field}
                field={field}
                busy={use.isPending}
                onUse={(value) => choose([{ field: field.field, value }])}
              />
            ))}
          </dl>
          {shown.some((field) => field.field === "CatalogueNumber") && (
            <p className={styles.quiet}>{t("lookUp.catalogueNote")}</p>
          )}
          {shown.length > 1 && (
            <button
              type="button"
              className={styles.button}
              disabled={use.isPending}
              onClick={() =>
                choose(shown.map((field) => ({ field: field.field, value: field.suggestions[0]?.value ?? "" })))
              }
            >
              {t("lookUp.useAll")}
            </button>
          )}
        </>
      )}
      {used !== null && (
        <p role="status" className={styles.quiet}>
          {used.changed ? t("lookUp.used", { count: used.count }) : t("lookUp.usedNothing")}
        </p>
      )}
      {errors.map((error, index) => (
        <p key={index} role="alert" className={styles.error}>
          {errorMessage(error)}
        </p>
      ))}
    </section>
  );
}

/** The fields with something to suggest, in the order the app gives them; the catalogue number last. */
function fieldsWithSuggestions(data: IdentifySuggestions): IdentifyFieldSuggestions[] {
  return [...data.fields, data.catalogueNumber].filter((field) => field.suggestions.length > 0);
}

function FieldRow({
  field,
  busy,
  onUse,
}: {
  field: IdentifyFieldSuggestions;
  busy: boolean;
  onUse: (value: string) => void;
}) {
  const { t } = useTranslation("identify");
  const name = t(`lookUp.field.${field.field}`);
  return (
    <div className={styles.field}>
      <dt className={styles.name}>{name}</dt>
      <dd className={styles.value}>
        <p className={styles.quiet}>
          {field.current === null || field.current === ""
            ? t("lookUp.currentNone")
            : t("lookUp.current", { value: field.current })}
        </p>
        <ul className={styles.suggestions}>
          {field.suggestions.map((suggestion) => (
            <li key={suggestion.value} className={styles.suggestion}>
              <span className={styles.suggested}>{suggestion.value}</span>
              <span className={styles.quiet}>{sourcesText(suggestion.sources, t)}</span>
              <button
                type="button"
                className={styles.use}
                aria-label={t("lookUp.useValue", { value: suggestion.value, field: name })}
                disabled={busy}
                onClick={() => onUse(suggestion.value)}
              >
                {t("lookUp.use")}
              </button>
            </li>
          ))}
        </ul>
      </dd>
    </div>
  );
}

/** Every source in plain words, each once. */
function sourcesText(sources: IdentifySource[], t: T): string {
  return [...new Set(sources.map((source) => sourceText(source, t)))].join(t("lookUp.sourceSeparator"));
}

function sourceText(source: IdentifySource, t: T): string {
  switch (source.kind) {
    case "tags":
      return t("lookUp.source.tags");
    case "fileName":
      return t("lookUp.source.fileName");
    case "sibling":
      return t("lookUp.source.sibling");
    case "online":
      return source.via === null || source.via === ""
        ? t("lookUp.source.online", { service: source.service })
        : t("lookUp.source.onlineVia", { service: source.service, via: source.via });
    default:
      // A source this build doesn't know: say what the data says.
      return t("lookUp.source.other", { kind: (source as { kind: string }).kind });
  }
}

/** The online sources behind the suggestions, for "Found ... from X". */
function onlineSources(data: IdentifySuggestions | undefined, t: T): string {
  if (data === undefined) return "";
  const online = [...data.fields, data.catalogueNumber]
    .flatMap((field) => field.suggestions.flatMap((suggestion) => suggestion.sources))
    .filter((source) => source.kind === "online");
  return sourcesText(online, t);
}

function statusLine(status: IdentifyLookupStatus | null | undefined, data: IdentifySuggestions | undefined, t: T): string {
  if (status == null) return "";
  const when = status.when === null ? "" : whenText(status.when);
  const reason = status.reason === null ? "" : reasonText(status.reason, t);
  switch (status.state) {
    case "never":
      return t("lookUp.status.never");
    case "queued":
      return t("lookUp.status.queued");
    case "running":
      return t("lookUp.status.running");
    case "found": {
      const source = onlineSources(data, t);
      return source === ""
        ? t("lookUp.status.found", { count: status.found, when })
        : t("lookUp.status.foundFrom", { count: status.found, source, when });
    }
    case "nothing":
      return reason === "" ? t("lookUp.status.nothing", { when }) : t("lookUp.status.nothingWhy", { when, reason });
    case "failed":
      return reason === "" ? t("lookUp.status.failed", { when }) : t("lookUp.status.failedWhy", { when, reason });
    case "skipped":
      return reason === "" ? t("lookUp.status.skipped") : t("lookUp.status.skippedWhy", { reason });
    default:
      return "";
  }
}

function reasonText(code: string, t: T): string {
  return isReason(code) ? t(`lookUp.reason.${code}`) : t("lookUp.reason.other", { code });
}

/** A time from the app in the user's own way of writing it. The app's times are UTC, with or without the zone marked. */
function whenText(when: string): string {
  const stamped = /^\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?$/.test(when) ? `${when.replace(" ", "T")}Z` : when;
  const date = new Date(stamped);
  return Number.isNaN(date.getTime()) ? when : date.toLocaleString();
}

function askedLine(asked: { selected: number; sent: number; queued: boolean }, t: T): string {
  if (!asked.queued) return t("lookUp.asked.none");
  const left = asked.selected - asked.sent;
  return left > 0
    ? t("lookUp.asked.someLeftOut", { count: asked.sent, left })
    : t("lookUp.asked.queued", { count: asked.sent });
}
