import i18n, { englishResources } from "../i18n";

type Messages = { [key: string]: string | Messages };

/** A `{{name}}`, `{{name, format}}` or `{{- name}}`: kept whole in a marker. */
const PLACEHOLDER = /\{\{[^}]*\}\}/g;

/**
 * Every English text with its wording replaced by a marker that names its
 * key (`[[send:review.new_one]] {{count, number}}`), placeholders kept so
 * interpolation still works. Proves no test depends on the wording: with
 * the markers in place the whole suite must still pass (`npm run
 * test:markers`).
 */
export function marked(messages: Messages, namespace: string, path = ""): Messages {
  return Object.fromEntries(
    Object.entries(messages).map(([name, value]) => {
      const key = path === "" ? name : `${path}.${name}`;
      if (typeof value !== "string") return [name, marked(value, namespace, key)];
      const kept = value.match(PLACEHOLDER) ?? [];
      return [name, [`[[${namespace}:${key}]]`, ...kept].join(" ")];
    }),
  );
}

/** Swaps the loaded English texts for markers. Call once, before any test runs. */
export function applyCopyMarkers() {
  for (const [namespace, messages] of Object.entries(englishResources)) {
    i18n.addResourceBundle("en", namespace, marked(messages, namespace), true, true);
  }
}
