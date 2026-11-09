import type { useTranslation } from "react-i18next";
import type { IdentifySource } from "../bindings";
import { MUSICBRAINZ_SERVICE } from "./useIdentify";

export type T = ReturnType<typeof useTranslation<"identify">>["t"];

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

/** Every source in plain words, each once. */
export function sourcesText(sources: IdentifySource[], t: T): string {
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

export function reasonText(code: string, t: T): string {
  return isReason(code) ? t(`lookUp.reason.${code}`) : t("lookUp.reason.other", { code });
}
