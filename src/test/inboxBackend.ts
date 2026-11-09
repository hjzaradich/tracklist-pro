import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  DoneForYou,
  InboxCount,
  InboxFilter,
  InboxItem,
  InboxLookupStatus,
  InboxOutcome,
  InboxRefusal,
  Label,
  LabelCounts,
  MusicFolder,
  Skipped,
} from "../bindings";
import { quietAnswer } from "./quietCommands";

/** An Inbox item the way the Rust side sends it; the file name is data of the test. */
export function inboxItem(id: number, fields: Partial<InboxItem> & { fileName?: string } = {}): InboxItem {
  return {
    id,
    fileId: id + 1000,
    musicFolderId: 1,
    batchId: 1,
    batchKind: "loose",
    batchLabel: null,
    status: "new",
    arrivedAt: "2026-11-01T10:00:00.000Z",
    decidedAt: null,
    present: true,
    onlineOnly: false,
    label: "new_to_you",
    reason: { key: "inbox:reason.newToYou", params: {}, source: "fact" },
    libraryTrackId: null,
    fileName: `file ${id}.mp3`,
    ...fields,
  } as InboxItem;
}

const REASONS: Record<Label, string> = {
  checking: "inbox:reason.checking",
  new_to_you: "inbox:reason.newToYou",
  already_have: "inbox:reason.alreadyHave",
  better_file: "review:reason.hasArt",
  other_cut: "inbox:reason.otherCut",
  rework: "inbox:reason.rework",
};

/** An item with the label and the matching reason. */
export function labeled(id: number, label: Label, fields: Partial<InboxItem> & { fileName?: string } = {}) {
  return inboxItem(id, { label, reason: { key: REASONS[label], params: {}, source: "fact" }, ...fields });
}

export function inboxFolder(id: number, path: string): MusicFolder {
  return {
    id,
    role: "inbox",
    watch: true,
    path,
    online: true,
    folderFound: true,
    volumeLabel: "",
    addedAt: "2026-10-01T10:00:00.000Z",
    walkedAt: null,
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
  };
}

const BY_LABEL: Record<Label, keyof LabelCounts> = {
  checking: "checking",
  new_to_you: "newToYou",
  already_have: "alreadyHave",
  better_file: "betterFile",
  other_cut: "otherCut",
  rework: "rework",
};

/**
 * Stands in for the Rust side of the Inbox: the items (the label filter and
 * the `new` status are honoured), the count and folders they come to, and
 * the five actions, each recording what it was asked (`calls`) and moving
 * the items on. `refused` makes an item's actions refuse with that reason;
 * `lowBitrate` makes adding an item need the go-ahead until it is confirmed;
 * `skipOnAddAll` is what "Add all" leaves out; `lookUps` are the look-up
 * statuses of the files asked about.
 */
export function inboxBackend(options: {
  items: InboxItem[];
  folders?: MusicFolder[];
  waitingForFirstLook?: number[];
  refused?: Record<number, InboxRefusal>;
  lowBitrate?: Record<number, number>;
  skipOnAddAll?: Skipped[];
  doneForYou?: DoneForYou[];
  /** Where each file's online look-up stands; a file not listed has no status. */
  lookUps?: InboxLookupStatus[];
}) {
  const state = {
    items: options.items,
    folders: options.folders ?? [],
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
    nextOperation: 1,
    lookUps: options.lookUps ?? [],
  };
  const called = (cmd: string) => state.calls.filter((call) => call.cmd === cmd);

  const fresh = () => state.items.filter((item) => item.status === "new");
  const count = (): InboxCount => {
    const byLabel: LabelCounts = { checking: 0, newToYou: 0, alreadyHave: 0, betterFile: 0, otherCut: 0, rework: 0 };
    for (const item of fresh()) byLabel[BY_LABEL[item.label]] += 1;
    return {
      new: fresh().length,
      alreadyThere: 0,
      byLabel,
      folders: state.folders
        .filter((f) => f.role === "inbox")
        .map((f) => ({ musicFolderId: f.id, waitingForFirstLook: (options.waitingForFirstLook ?? []).includes(f.id) })),
    };
  };

  const settle = (ids: number[], status: InboxItem["status"]): InboxOutcome => {
    for (const id of ids) {
      const reason = options.refused?.[id];
      if (reason !== undefined) return { outcome: "refused", itemId: id, reason };
    }
    for (const item of state.items) if (ids.includes(item.id)) item.status = status;
    return { outcome: "done", operationId: state.nextOperation++, items: ids, skipped: [] };
  };

  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      state.calls.push({ cmd, args });
      switch (cmd) {
        case "inbox_items": {
          const filter = args.filter as InboxFilter;
          return fresh().filter((item) => filter.label == null || item.label === filter.label);
        }
        case "identify_inbox_status": {
          const wanted = args.fileIds as number[];
          return state.lookUps.filter((status) => wanted.includes(status.fileId));
        }
        case "inbox_done_for_you":
          return options.doneForYou ?? [];
        case "inbox_count":
          return count();
        case "onedrive_music_folders":
          return [];
        case "music_folders":
          return state.folders;
        case "inbox_add": {
          const ids = args.itemIds as number[];
          for (const id of ids) {
            const kbps = options.lowBitrate?.[id];
            if (kbps !== undefined && args.confirmLowBitrate !== true) {
              return { outcome: "needsConfirmation", itemId: id, warning: { code: "lowBitrate", kbps } };
            }
          }
          return settle(ids, "added");
        }
        case "inbox_add_batch": {
          const skipped = options.skipOnAddAll ?? [];
          const ids = fresh()
            .filter((item) => item.batchId === args.batchId)
            .map((item) => item.id)
            .filter((id) => !skipped.some((s) => s.itemId === id));
          const done = settle(ids, "added");
          return done.outcome === "done" ? { ...done, skipped } : done;
        }
        case "inbox_not_now":
          return settle(args.itemIds as number[], "not_now");
        case "inbox_hide":
          return settle(args.itemIds as number[], "hidden");
        case "inbox_clear":
          return settle(args.itemIds as number[], "hidden");
        case "add_music_folder": {
          const added = inboxFolder(state.folders.length + 10, args.path as string);
          state.folders.push(added);
          return added;
        }
        case "remove_music_folder":
          state.folders = state.folders.filter((f) => f.id !== args.id);
          return null;
        default:
          return quietAnswer(cmd);
      }
    },
    { shouldMockEvents: true },
  );
  return { state, called };
}
