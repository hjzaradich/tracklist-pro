// Tests for scripts/preview.mjs: what `npm run design:reset` may delete.
// Every test works in a temp folder; none touches the real preview folder.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, describe, it } from "node:test";
import { fileURLToPath } from "node:url";

import {
  claim,
  clearSample,
  isPreviewFolder,
  isSeeded,
  MARKER,
  reset,
  ROOT,
  SEEDED,
} from "./preview.mjs";

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tlp-preview-test-"));
after(() => fs.rmSync(scratch, { recursive: true, force: true }));

let n = 0;
function folder() {
  return path.join(scratch, `case-${n++}`);
}

describe("claim", () => {
  it("makes a new folder the preview's, with its marker", () => {
    const root = folder();
    claim(root);
    assert.ok(isPreviewFolder(root));
    assert.ok(fs.existsSync(path.join(root, MARKER)));
  });

  it("takes an empty folder", () => {
    const root = folder();
    fs.mkdirSync(root);
    claim(root);
    assert.ok(isPreviewFolder(root));
  });

  it("refuses a folder that holds anything and has no marker", () => {
    const root = folder();
    fs.mkdirSync(root);
    fs.writeFileSync(path.join(root, "mine.txt"), "keep");
    assert.throws(() => claim(root), /isn't the preview's folder/);
    assert.deepEqual(fs.readdirSync(root), ["mine.txt"]);
  });
});

describe("reset", () => {
  it("deletes the marked folder and everything in it, and only that", () => {
    const parent = folder();
    const root = path.join(parent, "preview");
    const sibling = path.join(parent, "neighbour.txt");
    fs.mkdirSync(root, { recursive: true });
    fs.writeFileSync(sibling, "keep");
    claim(root);
    fs.mkdirSync(path.join(root, "data"));
    fs.writeFileSync(path.join(root, "data", "x.db"), "");
    assert.equal(reset(root), true);
    assert.ok(!fs.existsSync(root));
    assert.ok(fs.existsSync(sibling));
  });

  it("refuses, and deletes nothing, without the marker", () => {
    const root = folder();
    fs.mkdirSync(path.join(root, "data"), { recursive: true });
    fs.writeFileSync(path.join(root, "data", "real.db"), "keep");
    assert.throws(() => reset(root), /Nothing was deleted/);
    assert.ok(fs.existsSync(path.join(root, "data", "real.db")));
  });

  it("does nothing for a folder that isn't there", () => {
    assert.equal(reset(folder()), false);
  });

  it("refuses a link to somewhere else, even with a marker behind it", () => {
    const target = folder();
    fs.mkdirSync(target);
    fs.writeFileSync(path.join(target, MARKER), "");
    fs.writeFileSync(path.join(target, "keep.txt"), "keep");
    const link = folder();
    fs.symlinkSync(target, link, "junction");
    assert.throws(() => reset(link));
    assert.ok(fs.existsSync(path.join(target, "keep.txt")));
  });

  it("removes a junction inside the folder as a link and leaves its target alone", () => {
    const outside = folder();
    fs.mkdirSync(path.join(outside, "deep"), { recursive: true });
    fs.writeFileSync(path.join(outside, "keep.txt"), "keep");
    fs.writeFileSync(path.join(outside, "deep", "also.txt"), "keep");
    const root = folder();
    claim(root);
    fs.mkdirSync(path.join(root, "data"));
    fs.symlinkSync(outside, path.join(root, "music"), "junction");
    assert.equal(reset(root), true);
    assert.ok(!fs.existsSync(root));
    assert.equal(fs.readFileSync(path.join(outside, "keep.txt"), "utf8"), "keep");
    assert.equal(fs.readFileSync(path.join(outside, "deep", "also.txt"), "utf8"), "keep");
  });

  it("clearing the sample also leaves a junction's target alone", () => {
    const outside = folder();
    fs.mkdirSync(outside);
    fs.writeFileSync(path.join(outside, "keep.txt"), "keep");
    const root = folder();
    claim(root);
    fs.symlinkSync(outside, path.join(root, "documents"), "junction");
    clearSample(root);
    assert.ok(!fs.existsSync(path.join(root, "documents")));
    assert.equal(fs.readFileSync(path.join(outside, "keep.txt"), "utf8"), "keep");
  });

  it(
    "a reset that stops part-way keeps the marker, so it can be run again",
    { skip: process.platform !== "win32" && "a working folder can be deleted elsewhere" },
    () => {
      const root = folder();
      claim(root);
      // `webview` sorts after the marker's name, so a reset that deleted
      // the marker before the rest (the first version) would have lost it
      // by the time this one stops it.
      assert.ok("webview" > MARKER);
      const busy = path.join(root, "webview");
      fs.mkdirSync(busy);
      fs.mkdirSync(path.join(root, "data"));
      fs.mkdirSync(path.join(root, "music"));
      // Windows won't remove a folder a process is working in, as when the
      // preview window is still open.
      const before = process.cwd();
      process.chdir(busy);
      try {
        assert.throws(() => reset(root));
        assert.ok(isPreviewFolder(root), "the marker is still there");
      } finally {
        process.chdir(before);
      }
      assert.equal(reset(root), true);
      assert.ok(!fs.existsSync(root));
    },
  );

  it("refuses a drive root and a relative path", () => {
    assert.throws(() => reset(path.parse(scratch).root));
    assert.throws(() => reset("preview"));
  });
});

describe("the seed marker", () => {
  it("a half-made sample is cleared, keeping the folder and its marker", () => {
    const root = folder();
    claim(root);
    fs.mkdirSync(path.join(root, "data"));
    fs.mkdirSync(path.join(root, "music"));
    assert.equal(isSeeded(root), false);
    clearSample(root);
    assert.deepEqual(fs.readdirSync(root), [MARKER]);
  });

  it("is seeded only once the last file is there", () => {
    const root = folder();
    claim(root);
    assert.equal(isSeeded(root), false);
    fs.writeFileSync(path.join(root, SEEDED), "");
    assert.equal(isSeeded(root), true);
  });
});

describe("the script's reach", () => {
  const text = fs.readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), "preview.mjs"),
    "utf8",
  );

  it("works on a fixed folder outside the repo and OneDrive", () => {
    assert.equal(ROOT, "C:\\dev\\tracklist-pro-preview");
    assert.ok(!/onedrive/i.test(ROOT));
  });

  it("never reads a path from the environment or the arguments", () => {
    // It writes the variable for the app it starts; it never reads it.
    assert.ok(!/process\.env\.TLP_PREVIEW_DIR/.test(text));
    assert.ok(!/process\.argv\[[3-9]\]/.test(text));
  });
});
