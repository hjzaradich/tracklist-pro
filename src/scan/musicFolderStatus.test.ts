import { describe, expect, it } from "vitest";
import type { MusicFolder } from "../bindings";
import i18n from "../i18n";
import { describeNote, folderNotes } from "./musicFolderStatus";

function folder(fields: Partial<MusicFolder> = {}): MusicFolder {
  return {
    id: 1,
    role: "scan",
    watch: false,
    path: String.raw`E:\DJ Music`,
    online: true,
    volumeLabel: "GIG USB",
    addedAt: "2026-09-29T10:00:00.000Z",
    walkedAt: "2026-09-29T10:05:00.000Z",
    unreadableFolders: 0,
    unreadableFiles: 0,
    onlineOnlyFiles: 0,
    ...fields,
  };
}

const t = i18n.getFixedT("en", "musicFolderStatus");
const words = (f: MusicFolder) => folderNotes(f).map((note) => describeNote(note, t));

describe("what a music folder says about itself", () => {
  it("says nothing when the last scan read everything and nothing is online only", () => {
    expect(words(folder())).toEqual([]);
  });

  it("says its drive isn't connected, naming the drive, and nothing else", () => {
    const offline = folder({ online: false, unreadableFolders: 3, onlineOnlyFiles: 2 });
    expect(words(offline)).toEqual(["Drive not connected (GIG USB)"]);
    expect(words(folder({ online: false, volumeLabel: "" }))).toEqual(["Drive not connected"]);
  });

  it("says it hasn't been scanned until a scan has walked it", () => {
    const fresh = folder({ walkedAt: null, unreadableFolders: null, unreadableFiles: null });
    expect(words(fresh)).toEqual(["Not scanned yet"]);
  });

  it("counts what the scan couldn't read and the online-only files, in singular and plural", () => {
    expect(words(folder({ unreadableFolders: 1, unreadableFiles: 1, onlineOnlyFiles: 1 }))).toEqual([
      "1 folder couldn't be read",
      "1 file couldn't be read",
      "1 file online only (not downloaded)",
    ]);
    expect(words(folder({ unreadableFolders: 3, unreadableFiles: 12, onlineOnlyFiles: 40 }))).toEqual([
      "3 folders couldn't be read",
      "12 files couldn't be read",
      "40 files online only (not downloaded)",
    ]);
  });
});
