// Builds the text of src/i18n/resources.gen.ts, the file that tells
// TypeScript which namespaces exist, so a mistyped namespace or key fails
// `npm run build`. Keys come straight from each JSON file, so the generated
// file changes only when a namespace file is added, removed or renamed.
//
// Pure (no Node or DOM), so both the generator script and the tests use it.

/** Where the generated file lives, relative to the repo root. */
export const RESOURCE_TYPES_PATH = "src/i18n/resources.gen.ts";

/** The command that regenerates it. */
export const RESOURCE_TYPES_COMMAND = "npm run i18n:types";

// A namespace is the file name of src/locales/en/<namespace>.json. `:` and `.`
// would clash with i18next's namespace and key separators.
const NAMESPACE_NAME = /^[A-Za-z][A-Za-z0-9_-]*$/;

export function renderResourceTypes(namespaces: readonly string[]): string {
  const bad = namespaces.filter((ns) => !NAMESPACE_NAME.test(ns));
  if (bad.length > 0) {
    throw new Error(
      `Namespace file names must be letters, digits, "-" or "_": ${bad.join(", ")}`,
    );
  }
  // Sorted, one line per namespace: two lanes adding different namespaces
  // rarely touch the same line, and a conflict is fixed by regenerating.
  const lines = [...new Set(namespaces)]
    .sort()
    .map((ns) => `  ${JSON.stringify(ns)}: typeof import("../locales/en/${ns}.json");`);
  return [
    `// Generated from src/locales/en/*.json by \`${RESOURCE_TYPES_COMMAND}\`. Don't edit.`,
    "// `npm run dev` regenerates it; `npm run build` and the tests fail when it's stale.",
    "",
    "export interface EnglishResources {",
    ...lines,
    "}",
    "",
  ].join("\n");
}
