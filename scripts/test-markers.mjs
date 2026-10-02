// Proves no frontend test depends on the wording of the UI text (1aF-3): runs
// the whole suite with every English text replaced by a marker that names its
// key (src/test/copyMarkers.ts). Any failure is a test that pins English
// instead of looking it up with tx() (src/test/tx.ts).
//
//   node scripts/test-markers.mjs [vitest arguments]     (npm run test:markers)
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const vitest = join(dirname(fileURLToPath(import.meta.url)), "..", "node_modules", "vitest", "vitest.mjs");
const result = spawnSync(process.execPath, [vitest, "run", ...process.argv.slice(2)], {
  stdio: "inherit",
  env: { ...process.env, VITE_COPY_MARKERS: "1" },
});
process.exit(result.status ?? 1);
