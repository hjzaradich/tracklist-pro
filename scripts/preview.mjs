// The development preview: a debug build of the app that opens a made-up
// sample library, so the screens can be seen (and redrawn as the styles
// change) without touching the real app data folder.
//
//   npm run design         seed the sample once, then start the app
//   npm run design:reset   delete the sample (the next start makes it again)
//
// Everything lives in one fixed folder, ROOT below. The app (src-tauri/src/
// preview.rs) holds the same path and refuses any other. This script only
// ever deletes inside ROOT, only when ROOT holds MARKER, and never takes a
// path from the environment or the command line.
//
//   ROOT\data\       the app data folder (the database)
//   ROOT\music\      the generated music folder
//   ROOT\documents\  where the preview looks for a rekordbox export
//   ROOT\webview\    the window's own browser profile

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = "C:\\dev\\tracklist-pro-preview";
export const MARKER = "tlp-preview-folder.txt";
export const SEEDED = "seeded.txt";

/** What a seed makes inside the folder, and a reset clears. */
const MADE = ["data", "music", "documents", "webview"];

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** The folder as a plain, real directory: not missing, not a link. */
function realDir(root) {
  let stat;
  try {
    stat = fs.lstatSync(root);
  } catch {
    return false;
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) return false;
  return path.resolve(fs.realpathSync(root)).toLowerCase() === path.resolve(root).toLowerCase();
}

/** Whether `root` is a folder this script made. */
export function isPreviewFolder(root) {
  if (!realDir(root)) return false;
  try {
    return fs.lstatSync(path.join(root, MARKER)).isFile();
  } catch {
    return false;
  }
}

function assertSafeRoot(root) {
  if (!path.isAbsolute(root) || path.parse(root).root === path.resolve(root)) {
    throw new Error(`${root} is not a folder this script may use`);
  }
}

/** Makes `root` the preview's folder: new, or empty, or already marked. */
export function claim(root) {
  assertSafeRoot(root);
  if (isPreviewFolder(root)) return;
  if (fs.existsSync(root)) {
    if (!realDir(root) || fs.readdirSync(root).length > 0) {
      throw new Error(
        `${root} exists and isn't the preview's folder (no ${MARKER}). Nothing was changed.`,
      );
    }
  } else {
    fs.mkdirSync(root, { recursive: true });
  }
  fs.writeFileSync(
    path.join(root, MARKER),
    "This folder belongs to the tracklist-pro development preview (npm run design).\n" +
      "Everything in it is made-up sample data. `npm run design:reset` deletes it.\n",
  );
}

/** Deletes `root` and everything in it, if it's the preview's folder. */
export function reset(root) {
  assertSafeRoot(root);
  if (!fs.existsSync(root)) return false;
  if (!isPreviewFolder(root)) {
    throw new Error(`${root} has no ${MARKER}; it isn't the preview's folder. Nothing was deleted.`);
  }
  fs.rmSync(root, { recursive: true, force: true });
  return true;
}

/** Clears what a seed made, keeping the folder and its marker. */
export function clearSample(root) {
  if (!isPreviewFolder(root)) {
    throw new Error(`${root} isn't the preview's folder.`);
  }
  for (const name of [...MADE, SEEDED]) {
    fs.rmSync(path.join(root, name), { recursive: true, force: true });
  }
}

export function isSeeded(root) {
  return isPreviewFolder(root) && fs.existsSync(path.join(root, SEEDED));
}

function run(command, args, options) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

/** Seeds the sample if it isn't there, then starts the app on it. */
export function start(root = ROOT) {
  claim(root);
  if (!isSeeded(root)) {
    clearSample(root);
    console.log("Making the sample library (the first build takes a while)...");
    const status = run("cargo", ["run", "--example", "preview_seed"], {
      cwd: path.join(repo, "src-tauri"),
    });
    if (status !== 0) {
      console.error("The sample library could not be made. Nothing was started.");
      return status;
    }
  }
  console.log("Starting the preview on the sample library...");
  return run("npm", ["run", "tauri", "dev"], {
    cwd: repo,
    shell: true,
    env: { ...process.env, TLP_PREVIEW_DIR: path.join(root, "data") },
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const command = process.argv[2] ?? "start";
  try {
    if (command === "start") {
      process.exitCode = start();
    } else if (command === "reset") {
      console.log(reset(ROOT) ? `Deleted ${ROOT}.` : `${ROOT} isn't there; nothing to delete.`);
    } else {
      console.error(`Unknown command: ${command} (use start or reset)`);
      process.exitCode = 2;
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
