// Release helper (ROADMAP 4.5). Three steps, each run by hand:
//
//   node scripts/release.mjs bump <X.Y.Z>
//   node scripts/release.mjs build --out <folder>
//   node scripts/release.mjs draft --out <folder>
//
// `bump` edits the version in the five places that hold it. `build` makes the
// Windows installer from a clean checkout at origin/main and writes its sha256.
// `draft` creates a DRAFT GitHub release with the installer attached. The script
// has no way to publish: the owner does that on GitHub. See docs/releasing.md.
// No dependencies of its own; spawn, fs and gh are injected so tests run
// without building or network.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = resolve(HERE, "..");

export const BUILD_COMMAND = ["npx", "tauri", "build", "--bundles", "nsis"];

const EOL = "\\r?\\n";

/** The five edits: file, pattern whose group 1 ends right before the version, expected match count. */
export const VERSION_PLACES = [
  { file: "package.json", re: new RegExp(`("name": "tracklist-pro",${EOL}(?:.*${EOL})*?\\s*"version": ")[^"]+`), count: 1 },
  { file: "package-lock.json", re: new RegExp(`("name": "tracklist-pro",${EOL}\\s*"version": ")[^"]+`, "g"), count: 2 },
  { file: "src-tauri/Cargo.toml", re: new RegExp(`(\\[package\\]${EOL}name = "tracklist-pro"${EOL}version = ")[^"]+`), count: 1 },
  { file: "src-tauri/Cargo.lock", re: new RegExp(`(name = "tracklist-pro"${EOL}version = ")[^"]+`), count: 1 },
  { file: "src-tauri/tauri.conf.json", re: /^(\s*"version": ")[^"]+/m, count: 1 },
];

const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;

export function parseVersion(text) {
  const m = SEMVER.exec(text ?? "");
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** True when a is greater than b (both X.Y.Z). */
export function isGreater(a, b) {
  const x = parseVersion(a);
  const y = parseVersion(b);
  for (let i = 0; i < 3; i++) {
    if (x[i] !== y[i]) return x[i] > y[i];
  }
  return false;
}

export function currentVersion(root, fs) {
  return JSON.parse(fs.readFileSync(join(root, "package.json"), "utf8")).version;
}

export function bump(root, next, deps) {
  const { fs, log } = deps;
  if (!parseVersion(next)) throw new Error(`"${next}" is not a version like 1.2.3.`);
  const current = currentVersion(root, fs);
  if (!isGreater(next, current)) {
    throw new Error(`Version ${next} is not greater than the current ${current}.`);
  }
  // Compute every edit first so a surprise in one file changes none.
  const edits = VERSION_PLACES.map((p) => {
    const text = fs.readFileSync(join(root, p.file), "utf8");
    let n = 0;
    const out = text.replace(p.re, (_m, head) => {
      n++;
      return head + next;
    });
    if (n !== p.count) {
      throw new Error(`${p.file}: expected ${p.count} version place(s), found ${n}. Nothing was changed.`);
    }
    return { p, out };
  });
  for (const { p, out } of edits) {
    fs.writeFileSync(join(root, p.file), out);
    log(`${p.file}: ${current} -> ${next}${p.count > 1 ? ` (${p.count} places)` : ""}`);
  }
}

const exeName = (version) => `tracklist-pro_${version}_x64-setup.exe`;

function git(deps, root, args) {
  return deps.run("git", args, { cwd: root });
}

function requireOk(result, what) {
  if (result.status !== 0) {
    throw new Error(`${what} failed${result.stderr ? `: ${String(result.stderr).trim()}` : "."}`);
  }
  return String(result.stdout ?? "").trim();
}

export function build(root, out, deps) {
  const { fs, log } = deps;
  if (!out) throw new Error("Give an output folder: --out <folder>.");
  const version = currentVersion(root, fs);
  const dest = join(out, exeName(version));
  if (fs.existsSync(dest)) throw new Error(`${dest} already exists. Delete it or pick another folder.`);

  const dirty = requireOk(git(deps, root, ["status", "--porcelain"]), "git status");
  if (dirty) throw new Error("The working tree is not clean. Commit or stash your changes first.");
  requireOk(git(deps, root, ["fetch", "-q", "origin"]), "git fetch");
  const head = requireOk(git(deps, root, ["rev-parse", "HEAD"]), "git rev-parse HEAD");
  const main = requireOk(git(deps, root, ["rev-parse", "origin/main"]), "git rev-parse origin/main");
  if (head !== main) throw new Error("HEAD is not at origin/main. Build from the latest merged main.");

  const [cmd, ...args] = BUILD_COMMAND;
  const built = deps.run(cmd, args, { cwd: root, stdio: "inherit" });
  if (built.status !== 0) throw new Error("The installer build failed.");

  const src = join(root, "src-tauri", "target", "release", "bundle", "nsis", exeName(version));
  if (!fs.existsSync(src)) throw new Error(`The build finished but ${src} is missing.`);
  fs.mkdirSync(out, { recursive: true });
  fs.copyFileSync(src, dest);
  const bytes = fs.readFileSync(dest);
  const sha = createHash("sha256").update(bytes).digest("hex");
  fs.writeFileSync(`${dest}.sha256`, `${sha}  ${exeName(version)}\n`);
  log(`File:   ${dest}`);
  log(`Size:   ${bytes.length} bytes`);
  log(`sha256: ${sha}`);
  return { dest, sha };
}

export function releaseNotes(version, sha) {
  return [
    `# tracklist-pro ${version}`,
    "",
    `sha256 of the installer: \`${sha}\``,
    "",
    "Install and remove: [docs/installing.md](https://github.com/hjzaradich/tracklist-pro/blob/main/docs/installing.md).",
    "",
    "The installer is unsigned: Windows SmartScreen -> More info -> Run anyway.",
    "",
    "This is a draft. Publishing is done by the owner on GitHub.",
    "",
  ].join("\n");
}

export function draft(root, out, deps) {
  const { fs, log } = deps;
  if (!out) throw new Error("Give an output folder: --out <folder>.");
  const version = currentVersion(root, fs);
  const tag = `v${version}`;
  const exe = join(out, exeName(version));
  const shaFile = `${exe}.sha256`;
  if (!fs.existsSync(exe) || !fs.existsSync(shaFile)) {
    throw new Error(`Missing ${exe} or its .sha256. Run the build step first.`);
  }
  const sha = String(fs.readFileSync(shaFile, "utf8")).trim().split(/\s+/)[0];

  if (requireOk(git(deps, root, ["tag", "-l", tag]), "git tag")) {
    throw new Error(`The tag ${tag} already exists locally.`);
  }
  if (requireOk(git(deps, root, ["ls-remote", "--tags", "origin", tag]), "git ls-remote")) {
    throw new Error(`The tag ${tag} already exists on GitHub.`);
  }
  if (deps.gh(["release", "view", tag]).status === 0) {
    throw new Error(`A release for ${tag} already exists.`);
  }

  const notes = join(out, `release-notes_${version}.md`);
  fs.writeFileSync(notes, releaseNotes(version, sha));
  // Always a draft. This script has no publish path on purpose.
  const made = deps.gh([
    "release", "create", tag, "--draft",
    "--title", `tracklist-pro ${version}`,
    "--notes-file", notes,
    exe, shaFile,
  ]);
  requireOk(made, "gh release create");
  log(`Draft release ${tag} created. Review it on GitHub; the owner publishes it there.`);
}

export function realDeps() {
  return {
    fs: { readFileSync, writeFileSync, existsSync, mkdirSync, copyFileSync },
    run: (cmd, args, opts = {}) =>
      spawnSync(cmd, args, { encoding: "utf8", shell: process.platform === "win32" && cmd === "npx", ...opts }),
    gh: (args) => spawnSync("gh", args, { encoding: "utf8" }),
    log: (m) => console.log(m),
  };
}

export function main(argv, deps = realDeps(), root = DEFAULT_ROOT) {
  const [cmd, ...rest] = argv;
  const outAt = rest.indexOf("--out");
  if (outAt >= 0 && !rest[outAt + 1]) throw new Error("--out needs a folder.");
  const out = outAt >= 0 ? resolve(rest[outAt + 1]) : null;
  if (cmd === "bump") return bump(root, rest[0], deps);
  if (cmd === "build") return build(root, out, deps);
  if (cmd === "draft") return draft(root, out, deps);
  throw new Error("Usage: release.mjs bump <X.Y.Z> | build --out <folder> | draft --out <folder>");
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(e.message);
    process.exitCode = 1;
  }
}
