import { describe, expect, it } from "vitest";
import { ERROR_KEYS, type ErrorKind, type IpcError } from "../bindings";
import { englishResources } from "../i18n";
import { errorMessage } from "../api/errors";
import { tx } from "../test/tx";

// What the music folder commands send when they refuse (src-tauri/src/scan/
// folders.rs), and the text each one is shown as.
const refusals: [IpcError, string][] = [
  [
    { kind: "notAFolder", params: { path: String.raw`E:\Nope` } },
    tx("musicFolders:notAFolder", { path: String.raw`E:\Nope` }),
  ],
  [{ kind: "badPath", params: { path: "Music" } }, tx("musicFolders:badPath", { path: "Music" })],
  [
    { kind: "alreadyAdded", params: { path: String.raw`E:\Music` } },
    tx("musicFolders:alreadyAdded", { path: String.raw`E:\Music` }),
  ],
  [
    { kind: "insideMusicFolder", params: { musicFolder: String.raw`E:\Music` } },
    tx("musicFolders:insideMusicFolder", { musicFolder: String.raw`E:\Music` }),
  ],
  [
    { kind: "containsMusicFolder", params: { musicFolder: String.raw`E:\Music` } },
    tx("musicFolders:containsMusicFolder", { musicFolder: String.raw`E:\Music` }),
  ],
  [{ kind: "musicFolderNotFound", params: {} }, tx("musicFolders:notFound")],
  [{ kind: "musicFolderInUse", params: {} }, tx("musicFolders:inUse")],
];

describe("music folder errors", () => {
  it("puts every refusal into its text, with the folder it names", () => {
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
