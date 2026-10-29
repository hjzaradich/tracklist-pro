import { mockIPC } from "@tauri-apps/api/mocks";
import type { AddedToCrate, Crate, OperationInfo, TableQuery, TableRow } from "../bindings";
import { pageOf, tableRow } from "./trackTableIpc";

/**
 * Stands in for the Rust side of crates, for tests of adding tracks to them
 * (1cB-13, 1cB-14): a track table of `rows`, the crates and what is in them,
 * the commands called (`calls`, in order, with their arguments), and a
 * history of operations the top bar's Undo takes back one at a time.
 *
 * A row with `libraryTrackId: null` is a track outside the Library: adding it
 * to a crate adds it to the Library too (1cB-23), in the same operation,
 * unless `refused` says "Add to Library" would refuse it (by track id, with
 * the reason). The sibling-name rule is the real one in miniature: two
 * things in one folder can't share a name (letter case ignored).
 * `hold("cmd")` makes the next calls of a command wait until the returned
 * function is called, so a test can look at the screen while it works.
 */
export type Refusal = "noFile" | "fileMissing" | "matchNotConfirmed" | "lowBitrate";

export function crateBackend(options: {
  rows: TableRow[];
  crates: Crate[];
  members?: Record<number, number[]>;
  refused?: Record<number, Refusal>;
}) {
  const state = {
    rows: options.rows,
    crates: [...options.crates],
    /** crate id -> the tracks (`recordingId`) in it. */
    members: new Map<number, Set<number>>(
      Object.entries(options.members ?? {}).map(([id, tracks]) => [Number(id), new Set(tracks)]),
    ),
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
    history: [] as { info: OperationInfo; undo: () => void }[],
    nextId: 100,
    nextLibraryTrack: 1000,
    holds: new Map<string, Promise<void>>(),
  };
  const called = (cmd: string) => state.calls.filter((call) => call.cmd === cmd);
  const refuse = (kind: string, params: Record<string, string> = {}) => {
    throw { kind, params };
  };
  const record = (
    kind: string,
    name: string,
    tracks: number,
    undo: () => void,
    also: { newToLibrary?: number; newCrate?: boolean } = {},
  ) => {
    const info: OperationInfo = {
      id: state.history.length + 1,
      kind,
      details: { name, from: null, tracks, newToLibrary: also.newToLibrary ?? null, newCrate: also.newCrate ?? null },
    };
    state.history.push({ info, undo });
    return info.id;
  };
  const recordingOf = (libraryTrackId: number) =>
    state.rows.find((row) => row.libraryTrackId === libraryTrackId)?.recordingId;
  const hold = (cmd: string) => {
    let release = () => {};
    state.holds.set(
      cmd,
      new Promise<void>((resolve) => {
        release = resolve;
      }),
    );
    return release;
  };

  /**
   * What adding `recordings` to `held` does: the tracks outside the Library
   * join it unless refused, and the whole thing is undone together.
   */
  const addTo = (held: Set<number>, recordings: number[]) => {
    const joined: number[] = [];
    const linked: TableRow[] = [];
    const leftOut = { noFile: 0, fileMissing: 0, matchNotConfirmed: 0, lowBitrate: 0 };
    let alreadyThere = 0;
    for (const recording of new Set(recordings)) {
      const row = state.rows.find((r) => r.recordingId === recording);
      if (row === undefined) return refuse("libraryTrackNotFound");
      if (row.libraryTrackId === null) {
        const why = options.refused?.[recording];
        if (why !== undefined) leftOut[why] += 1;
        else {
          linked.push(row);
          joined.push(recording);
        }
      } else if (held.has(recording)) alreadyThere += 1;
      else joined.push(recording);
    }
    for (const row of linked) {
      row.libraryTrackId = state.nextLibraryTrack++;
      row.inLibrary = true;
    }
    for (const recording of joined) held.add(recording);
    const done: Omit<AddedToCrate, "operationId"> = {
      added: joined.length,
      alsoAddedToLibrary: linked.length,
      alreadyThere,
      leftOut,
    };
    const undo = () => {
      for (const recording of joined) held.delete(recording);
      for (const row of linked) {
        row.libraryTrackId = null;
        row.inLibrary = false;
      }
    };
    return { done, joined, undo };
  };

  const handle = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "list_crates":
        return state.crates.map((c) => ({ ...c, trackCount: state.members.get(c.id)?.size ?? 0 }));
      case "track_table_page":
        return pageOf(state.rows, args.query as TableQuery);
      case "track_table_ids": {
        const query = args.query as TableQuery;
        return state.rows.slice(query.offset, query.offset + query.limit).map((row) => row.recordingId);
      }
      case "track_table_position":
        return null;
      case "track_tag_sources":
        return { inRekordbox: false, fields: [], confirmedEntry: null };
      case "crates_of_track":
        return state.crates
          .filter((c) => state.members.get(c.id)?.has(args.recordingId as number))
          .map((c) => c.id);
      case "create_crate": {
        const name = String(args.name).trim();
        const parent = (args.parent ?? null) as number | null;
        if (state.crates.some((c) => c.parentId === parent && c.name.toLowerCase() === name.toLowerCase())) {
          return refuse("crateNameTaken", { name });
        }
        const id = state.nextId++;
        state.crates.push({ id, parentId: parent, kind: "static", name, trackCount: 0 });
        record("create_crate", name, 0, () => {
          state.crates = state.crates.filter((c) => c.id !== id);
        });
        return id;
      }
      case "add_recordings_to_crate": {
        const id = args.id as number;
        const target = state.crates.find((c) => c.id === id);
        if (target === undefined || target.kind !== "static") return refuse("crateNotFound");
        const held = state.members.get(id) ?? new Set<number>();
        const { done, joined, undo } = addTo(held, args.recordings as number[]);
        state.members.set(id, held);
        const operationId =
          joined.length === 0
            ? null
            : record("add_to_crate", target.name, joined.length, undo, { newToLibrary: done.alsoAddedToLibrary });
        return { ...done, operationId };
      }
      case "add_recordings_to_new_crate": {
        const name = String(args.name).trim();
        if (name === "") return refuse("crateNameEmpty");
        if (state.crates.some((c) => c.parentId === null && c.name.toLowerCase() === name.toLowerCase())) {
          return refuse("crateNameTaken", { name });
        }
        // Everything is checked before anything is made: a refusal leaves no crate.
        const held = new Set<number>();
        const { done, joined, undo } = addTo(held, args.recordings as number[]);
        const id = state.nextId++;
        state.crates.push({ id, parentId: null, kind: "static", name, trackCount: 0 });
        state.members.set(id, held);
        const operationId = record(
          "add_to_crate",
          name,
          joined.length,
          () => {
            undo();
            state.crates = state.crates.filter((c) => c.id !== id);
            state.members.delete(id);
          },
          { newToLibrary: done.alsoAddedToLibrary, newCrate: true },
        );
        return { crateId: id, name, added: { ...done, operationId } };
      }
      case "remove_tracks_from_crate": {
        const id = args.id as number;
        const target = state.crates.find((c) => c.id === id);
        const held = state.members.get(id);
        if (target === undefined || held === undefined) return refuse("crateNotFound");
        const removed = (args.tracks as number[])
          .map(recordingOf)
          .filter((recording): recording is number => recording !== undefined && held.has(recording));
        for (const recording of removed) held.delete(recording);
        const operationId =
          removed.length === 0
            ? null
            : record("remove_from_crate", target.name, removed.length, () => {
                for (const recording of removed) held.add(recording);
              });
        return { changed: removed.length, skipped: (args.tracks as number[]).length - removed.length, operationId };
      }
      case "next_undo_operation":
        return { operation: state.history.at(-1)?.info ?? null, refusal: null };
      case "undo_last_operation": {
        const last = state.history.pop();
        if (last === undefined) return { status: "nothingToUndo" };
        last.undo();
        return { status: "undone", operation: last.info };
      }
      case "library_tracks":
      case "music_folders":
        return [];
      case "read_online_only_files":
        return false;
      case "key_notation":
        return "camelot";
      case "table_columns":
        return null;
      default:
        throw { kind: "internal", params: {} };
    }
  };

  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      state.calls.push({ cmd, args });
      const gate = state.holds.get(cmd);
      return gate === undefined ? handle(cmd, args) : gate.then(() => handle(cmd, args));
    },
    { shouldMockEvents: true },
  );
  return { state, called, hold };
}

/** Library crate and folder shorthands. */
export const crate = (id: number, name: string, parentId: number | null = null): Crate => ({
  id,
  parentId,
  kind: "static",
  name,
  trackCount: 0,
});
export const folder = (id: number, name: string, parentId: number | null = null): Crate => ({
  ...crate(id, name, parentId),
  kind: "folder",
});

/** `count` Library tracks, `Tune 0` first; the track at index n has id `100 + n`. */
export const libraryRows = (count: number): TableRow[] =>
  Array.from({ length: count }, (_, n) => tableRow(n, { title: `Tune ${n}`, recordingId: 100 + n, libraryTrackId: n + 1 }));

/** A track outside the Library. */
export const outsideRow = (n: number): TableRow =>
  tableRow(n, { title: `Stray ${n}`, recordingId: 100 + n, libraryTrackId: null, inLibrary: false });
