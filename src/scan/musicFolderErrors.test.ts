import { describe, expect, it } from "vitest";
import { ERROR_KEYS, type ErrorKind, type IpcError } from "../bindings";
import { englishResources } from "../i18n";
import { errorMessage } from "../api/errors";

// What the music folder commands send when they refuse (src-tauri/src/scan/
// folders.rs), and the owner-approved words for each.
const refusals: [IpcError, string][] = [
  [{ kind: "notAFolder", params: { path: String.raw`E:\Nope` } }, String.raw`Folder not found (E:\Nope)`],
  [{ kind: "badPath", params: { path: "Music" } }, "Can't use this location (Music)"],
  [
    { kind: "alreadyAdded", params: { path: String.raw`E:\Music` } },
    String.raw`Already a music folder (E:\Music)`,
  ],
  [
    { kind: "insideMusicFolder", params: { musicFolder: String.raw`E:\Music` } },
    String.raw`Already inside a music folder (E:\Music)`,
  ],
  [
    { kind: "containsMusicFolder", params: { musicFolder: String.raw`E:\Music` } },
    String.raw`Contains a music folder (E:\Music)`,
  ],
  [{ kind: "musicFolderNotFound", params: {} }, "Music folder not found"],
  [{ kind: "musicFolderInUse", params: {} }, "Can't remove: tracks use files in this folder"],
];

describe("music folder errors", () => {
  it("puts every refusal into the approved words, with the folder it names", () => {
    for (const [error, words] of refusals) {
      expect(errorMessage(error)).toBe(words);
    }
  });

  it("has no message in its namespace that no error kind uses", () => {
    const used = new Set<string>(Object.values(ERROR_KEYS));
    for (const name of Object.keys(englishResources.musicFolders)) {
      expect(used.has(`musicFolders:${name}`), name).toBe(true);
    }
    const kinds = refusals.map(([error]) => error.kind);
    const inNamespace = (Object.keys(ERROR_KEYS) as ErrorKind[]).filter((kind) =>
      ERROR_KEYS[kind].startsWith("musicFolders:"),
    );
    expect(inNamespace.sort()).toEqual([...kinds].sort());
  });
});
