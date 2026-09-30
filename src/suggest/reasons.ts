// The rules a suggestion's reasons follow before the UI shows it (ROADMAP §1,
// "No black boxes"). The Rust side enforces the same rules when it makes a
// suggestion (src-tauri/src/suggest); this is the last check before render.
import type { Reason } from "../bindings";
import i18n from "../i18n";

/** Why a suggestion can't be shown. */
export type ReasonProblem =
  // No reasons at all: if the app can't say why, it doesn't suggest.
  | "noReason"
  // A reason's source isn't exactly "fact" or "audioModel".
  | "badSource"
  // The audio model's opinion is never the only reason.
  | "onlyAudioModel"
  // The key has no text in the locale files (or names a group of keys), so
  // the raw key or an error would show.
  | "unknownKey"
  // A parameter isn't text or a finite number (e.g. NaN from Rust).
  | "badParam"
  // The text has a {{placeholder}} the reason gives no value for, so it
  // would show as "{{bpm}}".
  | "missingParam";

const SOURCES: readonly string[] = ["fact", "audioModel"] satisfies Reason["source"][];

/** A `{{name}}` or `{{name, format}}` or `{{- name}}` in locale text. */
const PLACEHOLDER = /\{\{-?\s*([^,}\s]+)[^}]*\}\}/g;

/**
 * The i18next options that turn a reason into text. Parameters go in
 * `replace`, so a parameter named like an i18next option (`keySeparator`,
 * `returnDetails`, …) is only ever filled into the text; `count` also picks
 * the plural form.
 */
export function reasonOptions(reason: Reason): { replace: Reason["params"]; count?: number } {
  const count = reason.params.count;
  return typeof count === "number" ? { replace: reason.params, count } : { replace: reason.params };
}

type LooseT = (key: string, options: object) => unknown;
// Reason keys come from Rust, so they can't be typed as literal keys; they're
// checked against the locale files here instead.
const looseT = i18n.t as unknown as LooseT;

/** The locale text a reason resolves to, before its parameters are filled in. */
function template(reason: Reason): unknown {
  const details = looseT(reason.key, { ...reasonOptions(reason), returnDetails: true }) as {
    usedLng: string;
    usedNS: string;
    exactUsedKey: string;
  };
  return i18n.getResource(details.usedLng, details.usedNS, details.exactUsedKey);
}

/**
 * The first rule the reasons break, or null if the suggestion can be shown.
 * Takes `unknown` because the value comes over IPC: a malformed payload must
 * be refused too, not trusted to match its type.
 */
export function reasonProblem(reasons: unknown): ReasonProblem | null {
  if (!Array.isArray(reasons) || reasons.length === 0) return "noReason";
  const list = reasons as Reason[];
  if (list.some((r) => !SOURCES.includes(r?.source))) return "badSource";
  if (list.every((r) => r.source === "audioModel")) return "onlyAudioModel";
  for (const reason of list) {
    const params: unknown = reason.params;
    if (typeof params !== "object" || params === null || Array.isArray(params)) return "badParam";
    const values = Object.values(params);
    if (values.some((p) => typeof p !== "string" && !Number.isFinite(p))) return "badParam";
    if (typeof reason.key !== "string") return "unknownKey";
    const text = template(reason);
    if (typeof text !== "string") return "unknownKey";
    for (const [, name] of text.matchAll(PLACEHOLDER)) {
      if (!Object.hasOwn(params, name)) return "missingParam";
    }
  }
  return null;
}
