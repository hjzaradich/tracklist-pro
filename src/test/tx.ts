import i18n from "../i18n";

/**
 * The English text of a locale key, exactly as the app shows it, for tests
 * to look for. A test never writes UI English itself: the wording is the
 * owner's to change (docs/copy-style.md), and a test that pins it would
 * break every time the wording does.
 *
 * `key` is `namespace:path.to.key` as in t(); plural forms are picked by
 * `count`, and `params` fill the {{placeholders}}. Values that are data in
 * the test (a title, a path, a number) go in `params` and stay literal there.
 *
 * Throws, instead of returning the key, when the key has no text, names a
 * group of texts rather than one, or leaves a {{placeholder}} unfilled, so a
 * typo can't turn into a test that looks for the wrong text and passes.
 */
export function tx(key: string, params: Record<string, string | number> = {}): string {
  if (!i18n.exists(key, params)) {
    throw new Error(`tx: no English text for "${key}"${params.count === undefined ? "" : ` (count ${params.count})`}`);
  }
  const lookup = i18n.t as (key: string, options: object) => unknown;
  // With returnObjects, a group of texts comes back as an object, not as an error string.
  const text = lookup(key, { ...params, returnObjects: true });
  if (typeof text !== "string" || text === key) {
    throw new Error(`tx: "${key}" is a group of texts, not one text`);
  }
  const unfilled = /\{\{[^}]*\}\}/.exec(text);
  if (unfilled) {
    throw new Error(`tx: "${key}" has ${unfilled[0]} but no value was passed for it`);
  }
  return text;
}
