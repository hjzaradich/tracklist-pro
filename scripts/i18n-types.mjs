// Regenerates src/i18n/resources.gen.ts from the files in src/locales/en/.
//
//   node scripts/i18n-types.mjs           write the file if it changed
//   node scripts/i18n-types.mjs --check   fail if it's stale (used by `npm run build`)
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  RESOURCE_TYPES_COMMAND,
  RESOURCE_TYPES_PATH,
  renderResourceTypes,
} from "../src/i18n/resourceTypes.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const localesDir = join(root, "src", "locales", "en");
const target = join(root, RESOURCE_TYPES_PATH);

const namespaces = readdirSync(localesDir)
  .filter((name) => name.endsWith(".json"))
  .map((name) => name.slice(0, -".json".length));
const expected = renderResourceTypes(namespaces);

let current = null;
try {
  current = readFileSync(target, "utf8").replace(/\r\n/g, "\n");
} catch {
  // Missing: treated as stale.
}

if (process.argv.includes("--check")) {
  if (current !== expected) {
    console.error(
      `${RESOURCE_TYPES_PATH} is out of date with src/locales/en/. Run \`${RESOURCE_TYPES_COMMAND}\` and commit the result.`,
    );
    process.exit(1);
  }
} else if (current !== expected) {
  writeFileSync(target, expected);
  console.log(`Updated ${RESOURCE_TYPES_PATH}`);
}
