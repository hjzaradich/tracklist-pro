// Writes public/third-party-licenses.json: every package shipped in the app,
// with its licence and the licence text found in the package. The installer
// build runs it (tauri.conf.json, beforeBuildCommand) so every installer
// carries a fresh list; dev and tests don't need the file (it is git-ignored).
//
//   node scripts/third-party-licenses.mjs
//
// Rust crates come from `cargo metadata`: the app's normal dependency graph
// for the Windows build, leaving out dev-only and build-only dependencies.
// npm packages come from package-lock.json, walked from the app's own
// `dependencies` (not devDependencies) through each package's dependencies.
// No dependencies of its own.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const TARGET = "x86_64-pc-windows-msvc";
const TEXT_FILE = /^(licen[cs]e|copying|notice|unlicense)/i;

/** The crates the app ships: nodes reachable from the root by normal (not dev, not build) edges. */
export function cratesFromMetadata(meta) {
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const stack = [meta.resolve.root];
  while (stack.length > 0) {
    const node = nodes.get(stack.pop());
    for (const dep of node?.deps ?? []) {
      const normal = dep.dep_kinds.some((k) => k.kind === null);
      if (!normal || seen.has(dep.pkg)) continue;
      seen.add(dep.pkg);
      stack.push(dep.pkg);
    }
  }
  return [...seen].map((id) => {
    const p = byId.get(id);
    return {
      name: p.name,
      version: p.version,
      license: p.license ?? "",
      repository: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
      dir: dirname(p.manifest_path),
    };
  });
}

/** The npm packages the app ships, from a v2/v3 lockfile; `dir` is relative to the lockfile's folder. */
export function npmFromLockfile(lock) {
  const packages = lock.packages ?? {};
  const find = (from, name) => {
    let base = from;
    for (;;) {
      const path = base === "" ? `node_modules/${name}` : `${base}/node_modules/${name}`;
      if (packages[path]) return path;
      if (base === "") return null;
      const cut = base.lastIndexOf("/node_modules/");
      base = cut < 0 ? "" : base.slice(0, cut);
    }
  };
  const seen = new Set();
  const stack = [""];
  while (stack.length > 0) {
    const from = stack.pop();
    const entry = packages[from];
    const own =
      from === ""
        ? entry.dependencies
        : { ...entry.dependencies, ...entry.optionalDependencies, ...entry.peerDependencies };
    for (const name of Object.keys(own ?? {})) {
      const path = find(from, name);
      if (path && !seen.has(path)) {
        seen.add(path);
        stack.push(path);
      }
    }
  }
  return [...seen].map((path) => {
    const p = packages[path];
    return {
      name: path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length),
      version: p.version,
      license: typeof p.license === "string" ? p.license : "",
      repository: null,
      dir: path,
    };
  });
}

/** Licence, copying and notice files at the top of a package folder, joined; "" when there are none. */
export function readLicenceText(dir) {
  if (!existsSync(dir)) return "";
  const files = readdirSync(dir, { withFileTypes: true })
    .filter((f) => f.isFile() && TEXT_FILE.test(f.name))
    .map((f) => f.name)
    .sort();
  return files
    .map((f) => readFileSync(join(dir, f), "utf8").replace(/\r\n/g, "\n").trim())
    .filter(Boolean)
    .join("\n\n");
}

/** The repository address an npm package names in its package.json, or null. */
export function readNpmRepository(dir) {
  try {
    const { repository, homepage } = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const url = typeof repository === "string" ? repository : repository?.url;
    return (url ?? homepage ?? null)?.replace(/^git\+/, "") ?? null;
  } catch {
    return null;
  }
}

/**
 * The finished list. `readText(entry)` gives a package's licence text and
 * `readRepo(entry)` its repository address. Equal texts are stored once; a
 * package with no text keeps its SPDX id and repository address.
 */
export function buildList(entries, readText, readRepo = () => null) {
  const texts = [];
  const index = new Map();
  const packages = [...entries]
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
    .filter((e, i, all) => i === 0 || e.name !== all[i - 1].name || e.version !== all[i - 1].version)
    .map((e) => {
      const text = readText(e);
      let id = null;
      if (text) {
        if (!index.has(text)) index.set(text, texts.push(text) - 1);
        id = index.get(text);
      }
      const repository = e.repository ?? readRepo(e) ?? `https://www.npmjs.com/package/${e.name}`;
      return {
        name: e.name,
        version: e.version,
        license: e.license,
        text: id,
        repository: id === null ? repository : null,
      };
    });
  return { texts, packages };
}

function main() {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const manifest = join(root, "src-tauri", "Cargo.toml");
  const run = spawnSync(
    "cargo",
    ["metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET, "--manifest-path", manifest],
    { encoding: "utf8", maxBuffer: 1 << 30 },
  );
  if (run.status !== 0) throw new Error(`cargo metadata --locked failed (Cargo.lock out of date, or cargo missing?); not changing Cargo.lock: ${run.stderr}`);
  const crates = cratesFromMetadata(JSON.parse(run.stdout)).map((c) => ({ ...c, kind: "crate" }));
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const npm = npmFromLockfile(lock).map((n) => ({ ...n, kind: "npm", dir: join(root, n.dir) }));
  const list = buildList(
    [...crates, ...npm],
    (e) => readLicenceText(e.dir),
    (e) => (e.kind === "npm" ? readNpmRepository(e.dir) : null),
  );
  mkdirSync(join(root, "public"), { recursive: true });
  writeFileSync(join(root, "public", "third-party-licenses.json"), JSON.stringify(list));
  console.log(`third-party licences: ${list.packages.length} packages, ${list.texts.length} distinct texts`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
