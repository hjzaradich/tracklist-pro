import type { TFunction } from "i18next";
import type { InboxItem, InboxOutcome, InboxRefusal, Reason } from "../bindings";
import { reasonOptions } from "../suggest/reasons";

/** A reason's text. Its key comes from Rust, so it can't be typed as a literal key. */
export function reasonText(t: TFunction<"inbox">, reason: Reason): string {
  const translate = t as unknown as (key: string, options: object) => string;
  // A cut or rework that has a name (radio, remix) says it; one without doesn't.
  const context = typeof reason.params.label === "string" ? "labeled" : undefined;
  return translate(reason.key, { ...reasonOptions(reason), context });
}

/** Why an action was refused, in words. */
export function refusalText(t: TFunction<"inbox">, refusal: InboxRefusal): string {
  return t(`refusal.${refusal}`);
}

/**
 * The file's name. The Inbox list does not carry one yet (the backend sends
 * the file's id only); this is the one place to read it from once it does.
 */
export function fileNameOf(item: InboxItem): string {
  return (item as InboxItem & { fileName?: string }).fileName ?? "";
}

/** What the last action on a row or a group came to, when it is worth showing. */
export type Result = InboxOutcome;
