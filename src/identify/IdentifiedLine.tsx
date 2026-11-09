import { useTranslation } from "react-i18next";
import type { Identified, InboxLookupStatus } from "../bindings";
import { reasonText, sourcesText, type T } from "./lookUpText";

/**
 * One quiet line under an Inbox file, from where its online look-up stands
 * (ROADMAP 2.2): what the file was identified as and by whom, or that nothing
 * was found, the look-up failed or was skipped, with the reason. Nothing for
 * a file never looked up. There is no Use here: the file isn't in the
 * Library yet, so there is no track to set.
 */
export function IdentifiedLine({ status, className }: { status: InboxLookupStatus | undefined; className: string }) {
  const { t } = useTranslation("identify");
  const text = status === undefined ? "" : lineText(status, t);
  return text === "" ? null : <p className={className}>{text}</p>;
}

function lineText(status: InboxLookupStatus, t: T): string {
  const reason = status.reason === null ? "" : reasonText(status.reason, t);
  switch (status.state) {
    case "queued":
    case "running":
      return t("inboxLine.looking");
    case "found": {
      const name = status.identified === null ? "" : nameText(status.identified, t);
      if (name === "") return t("inboxLine.found", { count: status.found });
      const source = status.identified === null ? "" : sourcesText(status.identified.sources, t);
      return source === ""
        ? t("inboxLine.identified", { name })
        : t("inboxLine.identifiedFrom", { name, source });
    }
    case "nothing":
      return reason === "" ? t("inboxLine.nothing") : t("inboxLine.nothingWhy", { reason });
    case "failed":
      return reason === "" ? t("inboxLine.failed") : t("inboxLine.failedWhy", { reason });
    case "skipped":
      return reason === "" ? t("inboxLine.skipped") : t("inboxLine.skippedWhy", { reason });
    default:
      return "";
  }
}

/** "Artist – Title (Version)", leaving out what wasn't found. */
function nameText(identified: Identified, t: T): string {
  const named = [identified.artist, identified.title].filter((part): part is string => part !== null && part !== "");
  if (named.length === 0) return "";
  const base = named.join(t("inboxLine.nameJoin"));
  return identified.mix === null || identified.mix === "" ? base : `${base}${t("inboxLine.mix", { mix: identified.mix })}`;
}
