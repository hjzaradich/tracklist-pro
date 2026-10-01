// The copy pipeline's logic (1aF-3): which UI strings the owner has approved
// and which are still proposed. Pure functions plus a thin file layer; the
// command-line front end is scripts/copy.mjs.
//
// Text lives in src/locales/en/<namespace>.json (the single store of text).
// Each namespace has a notes file, src/locales/notes/<namespace>.json, with
// one row per locale key (dotted path, e.g. "remove.title"):
//   purpose   what the string is for
//   where     the screen or component and the situation it appears in
//   approved  the exact text the owner approved (absent until then)
// A string is APPROVED only while its current value equals `approved`;
// otherwise it's PROPOSED. Editing an approved string therefore makes it
// proposed again by itself. The notes live outside the app bundle and
// outside what scripts/i18n-types.mjs reads.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const LOCALES_DIR = "src/locales/en";
export const NOTES_DIR = "src/locales/notes";

const ROW_FIELDS = ["purpose", "where", "approved"];

/** A `{{name}}`, `{{name, format}}` or `{{- name}}` in locale text. */
const PLACEHOLDER = /\{\{-?\s*([^,}\s]+)[^}]*\}\}/g;
const PLURAL_FORM = /^(.+)_(zero|one|two|few|many|other)$/;

/** Every leaf of a (nested) locale file as `dotted.key -> text`. Throws on a non-text leaf. */
export function flatten(messages, prefix = "") {
  const out = new Map();
  for (const [name, value] of Object.entries(messages)) {
    const key = prefix === "" ? name : `${prefix}.${name}`;
    if (typeof value === "string") {
      out.set(key, value);
    } else if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      for (const [inner, text] of flatten(value, key)) out.set(inner, text);
    } else {
      throw new Error(`${key}: a locale value must be text or a group of text`);
    }
  }
  return out;
}

/** The names of the `{{placeholders}}` in a text. */
export function placeholders(text) {
  return new Set([...text.matchAll(PLACEHOLDER)].map((match) => match[1]));
}

/** True if the text has a `{{` or `}}` that isn't part of a whole placeholder. */
function hasStrayBraces(text) {
  const rest = text.replace(PLACEHOLDER, "");
  return rest.includes("{{") || rest.includes("}}");
}

function sameSet(a, b) {
  return a.size === b.size && [...a].every((name) => b.has(name));
}

/** `{ stem, form }` for a plural key (`remove.conflicts_one`), or null. */
export function pluralParts(key) {
  const match = PLURAL_FORM.exec(key);
  return match ? { stem: match[1], form: match[2] } : null;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function jsonFiles(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.slice(0, -".json".length))
    .sort();
}

/**
 * Reads every locale namespace and its notes file.
 * Returns `{ namespace: { values: Map<key, text>, notes: object | null } }`;
 * `notes` is null when the namespace has no notes file, and a namespace that
 * has only a notes file has `values` null.
 */
export function readCopy(root = ROOT) {
  const copy = {};
  for (const ns of jsonFiles(join(root, LOCALES_DIR))) {
    copy[ns] = {
      values: flatten(readJson(join(root, LOCALES_DIR, `${ns}.json`))),
      notes: null,
    };
  }
  for (const ns of jsonFiles(join(root, NOTES_DIR))) {
    copy[ns] ??= { values: null, notes: null };
    copy[ns].notes = readJson(join(root, NOTES_DIR, `${ns}.json`));
  }
  return copy;
}

/** Writes one namespace's notes file (two-space JSON, LF, trailing newline). */
export function writeNotes(root, namespace, notes) {
  writeFileSync(join(root, NOTES_DIR, `${namespace}.json`), `${JSON.stringify(notes, null, 2)}\n`);
}

function isRow(row) {
  return typeof row === "object" && row !== null && !Array.isArray(row);
}

/** Placeholder and plural problems among `texts` (a Map of key -> text). */
function familyProblems(texts, label) {
  const problems = [];
  const families = new Map();
  for (const [key, text] of texts) {
    if (hasStrayBraces(text)) {
      problems.push(`${label} ${key}: has a "{{" or "}}" that isn't a whole {{placeholder}}`);
    }
    const parts = pluralParts(key);
    if (parts) {
      if (!families.has(parts.stem)) families.set(parts.stem, new Map());
      families.get(parts.stem).set(parts.form, text);
    }
  }
  for (const [stem, forms] of families) {
    const one = forms.get("one");
    const other = forms.get("other");
    if (one === undefined || other === undefined) continue;
    // The _one form may drop {{count}} ("One track"); nothing else may differ.
    const names = (text) => {
      const set = placeholders(text);
      set.delete("count");
      return set;
    };
    if (!sameSet(names(one), names(other))) {
      problems.push(
        `${label} ${stem}: the _one and _other forms use different {{placeholders}} (${[...names(one)].join(", ") || "none"} and ${[...names(other)].join(", ") || "none"})`,
      );
    }
  }
  return problems;
}

/**
 * Everything the CI check fails on, as readable lines (empty: all fine):
 * every locale key has a notes row and every row a key; purpose and where
 * are filled in; plural keys come in complete _one/_other pairs; the
 * {{placeholders}} of a plural family agree, in the current text and in the
 * approved text.
 */
export function checkCopy(copy) {
  const problems = [];
  for (const [ns, { values, notes }] of Object.entries(copy)) {
    if (values === null) {
      problems.push(`${ns}: notes file ${NOTES_DIR}/${ns}.json has no locale file ${LOCALES_DIR}/${ns}.json`);
      continue;
    }
    if (notes === null) {
      problems.push(`${ns}: no notes file (add ${NOTES_DIR}/${ns}.json with a row for each key)`);
      continue;
    }
    if (!isRow(notes)) {
      problems.push(`${ns}: the notes file must be an object of rows`);
      continue;
    }
    for (const key of values.keys()) {
      if (!(key in notes)) problems.push(`${ns}:${key}: no notes row`);
    }
    const approved = new Map();
    for (const [key, row] of Object.entries(notes)) {
      if (!values.has(key)) {
        problems.push(`${ns}:${key}: notes row without a locale key`);
        continue;
      }
      if (!isRow(row)) {
        problems.push(`${ns}:${key}: a notes row must be an object`);
        continue;
      }
      for (const field of Object.keys(row)) {
        if (!ROW_FIELDS.includes(field)) problems.push(`${ns}:${key}: unknown field "${field}"`);
      }
      for (const field of ["purpose", "where"]) {
        if (typeof row[field] !== "string" || row[field].trim() === "") {
          problems.push(`${ns}:${key}: "${field}" is empty`);
        }
      }
      if ("approved" in row) {
        if (typeof row.approved === "string") approved.set(key, row.approved);
        else problems.push(`${ns}:${key}: "approved" must be text`);
      }
    }
    for (const [key] of values) {
      const parts = pluralParts(key);
      if (!parts) continue;
      const sibling = (form) => values.has(`${parts.stem}_${form}`);
      if (parts.form === "one" && !sibling("other")) problems.push(`${ns}:${key}: no matching ${parts.stem}_other`);
      if (parts.form === "other" && !sibling("one")) problems.push(`${ns}:${key}: no matching ${parts.stem}_one`);
    }
    problems.push(...familyProblems(values, `${ns}:`));
    problems.push(...familyProblems(approved, `${ns}: approved`));
  }
  return problems;
}

/**
 * The strings still proposed, in locale-file order: no `approved`, or a
 * current value that differs from it. Each has `{ namespace, key, text,
 * purpose, where, approved, kind }`, `kind` being "new" (never approved) or
 * "changed" (edited since approval).
 */
export function proposedStrings(copy) {
  const out = [];
  for (const [namespace, { values, notes }] of Object.entries(copy)) {
    if (values === null) continue;
    for (const [key, text] of values) {
      const row = isRow(notes) && isRow(notes[key]) ? notes[key] : {};
      const approved = typeof row.approved === "string" ? row.approved : undefined;
      if (approved === text) continue;
      out.push({
        namespace,
        key,
        text,
        purpose: typeof row.purpose === "string" ? row.purpose : "",
        where: typeof row.where === "string" ? row.where : "",
        approved,
        kind: approved === undefined ? "new" : "changed",
      });
    }
  }
  return out;
}

/** Counts of all strings and of the proposed ones, for the summary line. */
export function counts(copy) {
  let total = 0;
  for (const { values } of Object.values(copy)) total += values === null ? 0 : values.size;
  return { total, proposed: proposedStrings(copy).length };
}

/**
 * Sets `approved` to the current text for each target, in memory. A target is
 * `namespace:key` or `{ allIn: namespace }`. Returns `{ changed: [namespace],
 * approved: count, problems: [] }`; any problem (unknown namespace or key, a
 * key with no notes row) means nothing was changed.
 */
export function approve(copy, targets) {
  const problems = [];
  const picks = [];
  for (const target of targets) {
    if (typeof target === "object") {
      const entry = copy[target.allIn];
      if (!entry?.values) {
        problems.push(`${target.allIn}: no such namespace`);
        continue;
      }
      for (const key of entry.values.keys()) picks.push([target.allIn, key]);
      continue;
    }
    const split = target.indexOf(":");
    const ns = split < 0 ? "" : target.slice(0, split);
    const key = split < 0 ? "" : target.slice(split + 1);
    if (split < 0 || key === "") {
      problems.push(`${target}: write it as <namespace>:<key>`);
    } else if (!copy[ns]?.values) {
      problems.push(`${target}: no such namespace`);
    } else if (!copy[ns].values.has(key)) {
      problems.push(`${target}: no such key`);
    } else {
      picks.push([ns, key]);
    }
  }
  for (const [ns, key] of picks) {
    if (!isRow(copy[ns].notes?.[key])) problems.push(`${ns}:${key}: no notes row to approve (run npm run copy:check)`);
  }
  if (problems.length > 0) return { changed: [], approved: 0, problems };
  const changed = new Set();
  let approved = 0;
  for (const [ns, key] of picks) {
    const row = copy[ns].notes[key];
    const text = copy[ns].values.get(key);
    if (row.approved === text) continue;
    row.approved = text;
    changed.add(ns);
    approved += 1;
  }
  return { changed: [...changed].sort(), approved, problems };
}

/** The proposed strings as readable text, for `copy:status` and `copy:gate`. */
export function formatProposed(list, total) {
  const lines = [];
  for (const item of list) {
    lines.push(`${item.namespace}:${item.key}  [${item.kind}]`);
    lines.push(`  text:    ${JSON.stringify(item.text)}`);
    if (item.approved !== undefined) lines.push(`  was:     ${JSON.stringify(item.approved)}`);
    lines.push(`  purpose: ${item.purpose || "(no notes row)"}`);
    lines.push(`  where:   ${item.where || "(no notes row)"}`);
  }
  if (lines.length > 0) lines.push("");
  lines.push(`${list.length} of ${total} strings proposed`);
  return lines.join("\n");
}
