import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildList, cratesFromMetadata, modelsFromFolder, npmFromLockfile, readLicenceText } from "./third-party-licenses.mjs";

const pkg = (name) => ({
  name,
  version: "1.0.0",
  id: name,
  license: "MIT",
  repository: `https://example.test/${name}`,
  manifest_path: `/reg/${name}/Cargo.toml`,
});
const edge = (target, kind) => ({ pkg: target, dep_kinds: [{ kind, target: null }] });

test("crates: dev-only and build-only dependencies are left out", () => {
  const meta = {
    packages: ["app", "normal", "deep", "devonly", "buildonly"].map(pkg),
    resolve: {
      root: "app",
      nodes: [
        { id: "app", deps: [edge("normal", null), edge("devonly", "dev"), edge("buildonly", "build")] },
        { id: "normal", deps: [edge("deep", null)] },
        { id: "deep", deps: [] },
        { id: "devonly", deps: [] },
        { id: "buildonly", deps: [] },
      ],
    },
  };
  assert.deepEqual(cratesFromMetadata(meta).map((c) => c.name).sort(), ["deep", "normal"]);
});

test("npm: devDependencies are left out, transitive ones kept", () => {
  const lock = {
    packages: {
      "": { dependencies: { a: "1" }, devDependencies: { d: "1" } },
      "node_modules/a": { version: "1.0.0", license: "MIT", dependencies: { b: "1" } },
      "node_modules/b": { version: "2.0.0", license: "ISC" },
      "node_modules/d": { version: "1.0.0", license: "MIT", dependencies: { e: "1" } },
      "node_modules/e": { version: "1.0.0", license: "MIT" },
    },
  };
  assert.deepEqual(npmFromLockfile(lock).map((p) => p.name).sort(), ["a", "b"]);
});

test("npm: a nested copy is found before the top-level one", () => {
  const lock = {
    packages: {
      "": { dependencies: { a: "1" } },
      "node_modules/a": { version: "1.0.0", dependencies: { b: "1" } },
      "node_modules/a/node_modules/b": { version: "1.0.0" },
      "node_modules/b": { version: "9.0.0" },
    },
  };
  assert.deepEqual(
    npmFromLockfile(lock).map((p) => p.dir).sort(),
    ["node_modules/a", "node_modules/a/node_modules/b"],
  );
});

test("equal licence texts are stored once", () => {
  const entries = ["x", "y", "z"].map((name) => ({ name, version: "1.0.0", license: "MIT", repository: null }));
  const list = buildList(entries, (e) => (e.name === "z" ? "Other text" : "Same text"));
  assert.equal(list.texts.length, 2);
  assert.equal(list.packages[0].text, list.packages[1].text);
  assert.notEqual(list.packages[0].text, list.packages[2].text);
});

test("a package with no text keeps its licence id and repository address", () => {
  const entries = [{ name: "x", version: "1.0.0", license: "Apache-2.0", repository: "https://example.test/x" }];
  const [only] = buildList(entries, () => "").packages;
  assert.equal(only.text, null);
  assert.equal(only.license, "Apache-2.0");
  assert.equal(only.repository, "https://example.test/x");
});

test("readLicenceText joins licence, copying and notice files and skips the rest", () => {
  const dir = mkdtempSync(join(tmpdir(), "licences-"));
  writeFileSync(join(dir, "LICENSE-MIT"), "Licence A\r\n");
  writeFileSync(join(dir, "NOTICE"), "Notice B\n");
  writeFileSync(join(dir, "README.md"), "Not a licence");
  assert.equal(readLicenceText(dir), "Licence A\n\nNotice B");
  assert.equal(readLicenceText(join(dir, "missing")), "");
  rmSync(dir, { recursive: true });
});

test("the same name and version is listed once", () => {
  const entry = { name: "x", version: "1.0.0", license: "MIT", repository: null };
  const list = buildList([entry, { ...entry }, { ...entry, version: "2.0.0" }], () => "T");
  assert.deepEqual(list.packages.map((p) => p.version), ["1.0.0", "2.0.0"]);
});

test("models: a folder with model.json is listed, with its licence and notice as the text", () => {
  const models = mkdtempSync(join(tmpdir(), "models-"));
  const dir = join(models, "m1");
  mkdirSync(dir);
  writeFileSync(join(dir, "model.json"), JSON.stringify({ name: "M1 (audio model)", version: "abc", license: "Apache-2.0", repository: "https://example.test/m1" }));
  writeFileSync(join(dir, "LICENSE"), "Apache text\n");
  writeFileSync(join(dir, "NOTICE.md"), "Credit\n");
  writeFileSync(join(dir, "m1.onnx"), "weights");
  mkdirSync(join(models, "no-json"));
  const [entry, ...rest] = modelsFromFolder(models);
  assert.equal(rest.length, 0);
  assert.equal(entry.name, "M1 (audio model)");
  assert.equal(entry.license, "Apache-2.0");
  const list = buildList([entry], (e) => readLicenceText(e.dir));
  assert.equal(list.texts[0], "Apache text\n\nCredit");
  assert.deepEqual(list.packages.map((p) => [p.name, p.version, p.license]), [["M1 (audio model)", "abc", "Apache-2.0"]]);
  assert.deepEqual(modelsFromFolder(join(models, "missing")), []);
  rmSync(models, { recursive: true });
});

test("the shipped CED-tiny folder is listed", () => {
  const dir = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "resources", "models");
  const ced = modelsFromFolder(dir).find((m) => m.name.startsWith("CED-tiny"));
  assert.equal(ced?.license, "Apache-2.0");
  assert.match(readLicenceText(ced.dir), /Apache License[\s\S]*Xiaomi/);
});
