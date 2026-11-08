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
import { useOnlineServicesStatus } from "../settings/useOnlineServices";
import { useSelectedTrack } from "../shell/selectedTrack";
import { useWhenUndone } from "../shell/undoneStore";
import styles from "./LookUpPanel.module.css";
import {
  DISCOGS_SERVICE,
  IDENTIFY_QUERY_KEY,
  MUSICBRAINZ_SERVICE,
  useIdentifySuggestions,
  useCover,
  useCoverReleases,
  useLookUpRecordings,
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
  const lookUpSelected = useLookUpRecordings();
  const use = useUseSuggestions();
  const selected = useSelectedTrack((state) => state.ids);
  const [asked, setAsked] = useState<{ count: number; queued: boolean } | null>(null);
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

  const askOne = () => {
    setAsked(null);
    lookUp.mutate([libraryTrackId], { onSuccess: (job) => setAsked({ count: 1, queued: job !== null }) });
  };
  // The selected tracks go as they are: the backend finds their Library tracks.
  const askSelected = () => {
    const count = selected.length;
    setAsked(null);
    lookUpSelected.mutate([...selected], { onSuccess: (job) => setAsked({ count, queued: job !== null }) });
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
  const errors = [suggestions.error, status.error, lookUp.error, lookUpSelected.error, use.error].filter((error) => error != null);
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
            disabled={off || lookUpSelected.isPending}
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
      <Cover libraryTrackId={libraryTrackId} album={albumName(suggestions.data)} />
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
          {shownSources(shown).some(isDiscogs) && <p className={styles.quiet}>{t("lookUp.discogsCredit")}</p>}
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

/** Every source of the suggestions shown. */
function shownSources(shown: IdentifyFieldSuggestions[]): IdentifySource[] {
  return shown.flatMap((field) => field.suggestions.flatMap((suggestion) => suggestion.sources));
}

const isDiscogs = (source: IdentifySource) => source.kind === "online" && source.service === DISCOGS_SERVICE;

/** The album the track has or is suggested, to name its cover. */
function albumName(data: IdentifySuggestions | undefined): string | null {
  const album = data?.fields.find((field) => field.field === "Album");
  const name = album?.current || album?.suggestions[0]?.value;
  return name ? name : null;
}

/** The picture types a `data:` address may carry here: anything else is not drawn. */
const IMAGE_TYPES = ["image/jpeg", "image/png", "image/webp", "image/gif"];

/**
 * The front cover of the track's first release, once Cover Art is on and the
 * kept look-up has a release. Drawn only from what the command returns (a
 * `data:` address), never from a web address. Nothing shows while Cover Art is
 * off or when the archive has no cover; a failure says so in a few words.
 */
function Cover({ libraryTrackId, album }: { libraryTrackId: number; album: string | null }) {
  const { t } = useTranslation("identify");
  const on = useOnlineServicesStatus().data?.coverArt === true;
  const releases = useCoverReleases(libraryTrackId, on);
  const first = on ? releases.data?.[0] : undefined;
  const cover = useCover(first);
  if (first === undefined) return null;
  if (cover.isPending) return <p className={styles.quiet}>{t("lookUp.cover.loading")}</p>;
  const answer = cover.data;
  if (cover.isError || answer?.kind === "failed") return <p className={styles.quiet}>{t("lookUp.cover.none")}</p>;
  if (answer?.kind !== "image" || !IMAGE_TYPES.includes(answer.mime)) return null;
  return (
    <img
      className={styles.cover}
      src={`data:${answer.mime};base64,${answer.data}`}
      alt={album === null ? t("lookUp.cover.alt") : t("lookUp.cover.altAlbum", { album })}
      width={96}
      height={96}
    />
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
      if (source.via !== null && source.via !== "") {
        return t("lookUp.source.onlineVia", { service: source.service, via: source.via });
      }
      // MusicBrainz with no service leading to it is the search by name.
      return source.service === MUSICBRAINZ_SERVICE
        ? t("lookUp.source.nameSearch", { service: source.service })
        : t("lookUp.source.online", { service: source.service });
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

function askedLine(asked: { count: number; queued: boolean }, t: T): string {
  return asked.queued ? t("lookUp.asked.queued", { count: asked.count }) : t("lookUp.asked.none");
}
