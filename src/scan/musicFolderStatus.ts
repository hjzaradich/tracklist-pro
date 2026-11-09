import type { TFunction } from "i18next";
import type { MusicFolder } from "../bindings";

/** One thing worth saying about a music folder, before it's put into words. */
export type FolderNote =
  | { kind: "offline"; label: string }
  | { kind: "folderNotFound" }
  | { kind: "notScanned" }
  | { kind: "unreadableFolders"; count: number }
  | { kind: "unreadableFiles"; count: number }
  | { kind: "onlineOnly"; count: number };

/**
 * What to say about a music folder, most important first. An offline
 * folder only says so: its counts are from before the drive went away. So
 * does a folder missing from a drive that is connected (moved, renamed or
 * deleted): its counts are from before it went.
 * A folder the scan read in full, with nothing online only, has nothing.
 */
export function folderNotes(folder: MusicFolder): FolderNote[] {
  if (!folder.online) return [{ kind: "offline", label: folder.volumeLabel }];
  if (!folder.folderFound) return [{ kind: "folderNotFound" }];
  if (folder.walkedAt == null) return [{ kind: "notScanned" }];
  const notes: FolderNote[] = [];
  if (folder.unreadableFolders) {
    notes.push({ kind: "unreadableFolders", count: folder.unreadableFolders });
  }
  if (folder.unreadableFiles) {
    notes.push({ kind: "unreadableFiles", count: folder.unreadableFiles });
  }
  if (folder.onlineOnlyFiles > 0) {
    notes.push({ kind: "onlineOnly", count: folder.onlineOnlyFiles });
  }
  return notes;
}

/** A note in words. */
export function describeNote(note: FolderNote, t: TFunction<"musicFolderStatus">): string {
  switch (note.kind) {
    case "offline":
      return note.label === "" ? t("offline") : t("offlineNamed", { label: note.label });
    case "folderNotFound":
      return t("folderNotFound");
    case "notScanned":
      return t("notScanned");
    case "unreadableFolders":
      return t("unreadableFolders", { count: note.count });
    case "unreadableFiles":
      return t("unreadableFiles", { count: note.count });
    case "onlineOnly":
      return t("onlineOnly", { count: note.count });
  }
}
