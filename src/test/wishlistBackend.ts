import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  WishItem,
  WishlistOutcome,
  WishlistRefusal,
  WishlistSummary,
  WishMatch,
  WishSkippedLine,
} from "../bindings";
import { quietAnswer } from "./quietCommands";

/** A result the way the Rust side sends it: the track is missing. */
export const MISSING: WishMatch = {
  kind: "missing",
  reason: { key: "wishlist:reason.missing", params: {}, source: "fact" },
  recordingId: null,
  inLibrary: false,
};

/** A result: the track is in the Library as `have`. */
export function owned(have: string): WishMatch {
  return {
    kind: "owned",
    reason: { key: "wishlist:reason.ownedLibrary", params: { have }, source: "fact" },
    recordingId: 1,
    inLibrary: true,
  };
}

/** A wishlist item the way the Rust side sends it; names are data of the test. */
export function wishItem(id: number, fields: Partial<WishItem> = {}): WishItem {
  return {
    id,
    wishlistId: 1,
    position: id,
    artist: `Artist ${id}`,
    title: `Title ${id}`,
    version: null,
    isrc: null,
    bought: false,
    notes: null,
    acquiredFileId: null,
    acquiredAt: null,
    result: MISSING,
    ...fields,
  };
}

export interface FakeWishlist {
  id: number;
  name: string;
  source: "paste" | "csv";
  items: WishItem[];
}

function summaryOf(list: FakeWishlist): WishlistSummary {
  const count = (kind: WishMatch["kind"]) => list.items.filter((i) => i.result.kind === kind).length;
  return {
    id: list.id,
    name: list.name,
    source: list.source,
    createdAt: "2026-11-01T10:00:00.000Z",
    total: list.items.length,
    owned: count("owned"),
    ownedOtherCut: count("ownedOtherCut"),
    rework: count("rework"),
    missing: count("missing"),
    bought: list.items.filter((i) => i.bought).length,
  };
}

/**
 * Stands in for the Rust side of wishlists: the feature switch, the lists
 * and their items, and the commands, each recording what it was asked
 * (`calls`). A pasted line "A - B" makes an item (missing, or owned when
 * `ownedTitles` has its title); a line with no " - " is skipped, as the real
 * parser does. `csvRows` is what any CSV file holds. `refuse` makes the next
 * create command refuse.
 */
export function wishlistBackend(
  options: {
    on?: boolean;
    lists?: FakeWishlist[];
    ownedTitles?: string[];
    csvRows?: { artist: string; title: string }[];
    refuse?: WishlistRefusal;
  } = {},
) {
  const state = {
    on: options.on ?? true,
    lists: options.lists ?? [],
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
    nextId: 100,
    nextOperation: 1,
  };
  const called = (cmd: string) => state.calls.filter((call) => call.cmd === cmd);

  const done = (wishlistId: number | null, added = 0, skipped: WishSkippedLine[] = []): WishlistOutcome => ({
    outcome: "done",
    operationId: state.nextOperation++,
    wishlistId,
    added,
    skipped,
    skippedTotal: skipped.length,
  });
  const find = (itemId: unknown) =>
    state.lists.flatMap((list) => list.items).find((item) => item.id === itemId);
  const result = (title: string): WishMatch => (options.ownedTitles?.includes(title) ? owned(title) : MISSING);

  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      state.calls.push({ cmd, args });
      switch (cmd) {
        case "feature_enabled":
          return state.on;
        case "set_feature_enabled":
          state.on = args.on as boolean;
          return null;
        case "wishlists_list":
          return { outcome: "lists", lists: state.lists.map(summaryOf) };
        case "wishlist_items": {
          const list = state.lists.find((l) => l.id === args.wishlistId);
          return { outcome: "items", items: list?.items ?? [] };
        }
        case "wishlist_create_from_paste":
        case "wishlist_create_from_csv": {
          if (options.refuse !== undefined) return { outcome: "refused", reason: options.refuse };
          const rows: { artist: string; title: string }[] = [];
          const skipped: WishSkippedLine[] = [];
          if (cmd === "wishlist_create_from_csv") {
            rows.push(...(options.csvRows ?? []));
          } else {
            (args.pasted as string).split("\n").forEach((text, index) => {
              if (text.trim() === "") return;
              const [artist, title] = text.split(" - ");
              if (title === undefined) skipped.push({ line: index + 1, text, reason: "noSeparator" });
              else rows.push({ artist, title });
            });
          }
          const id = state.nextId++;
          state.lists.push({
            id,
            name: args.name as string,
            source: cmd === "wishlist_create_from_csv" ? "csv" : "paste",
            items: rows.map((row, index) =>
              wishItem(id * 10 + index, { wishlistId: id, ...row, result: result(row.title) }),
            ),
          });
          return done(id, rows.length, skipped);
        }
        case "wishlist_delete":
          state.lists = state.lists.filter((l) => l.id !== args.wishlistId);
          return done(null);
        case "wishlist_remove_item":
          for (const list of state.lists) list.items = list.items.filter((i) => i.id !== args.itemId);
          return done(null);
        case "wishlist_set_bought": {
          const item = find(args.itemId);
          if (item) item.bought = args.bought as boolean;
          return done(null);
        }
        case "wishlist_set_notes": {
          const item = find(args.itemId);
          if (item) item.notes = args.notes as string | null;
          return done(null);
        }
        default:
          return quietAnswer(cmd);
      }
    },
    { shouldMockEvents: true },
  );
  return { state, called };
}
