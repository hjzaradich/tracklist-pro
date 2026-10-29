import { mockIPC } from "@tauri-apps/api/mocks";
import type { Crate, OperationInfo, TableQuery, TableRow } from "../bindings";
import { pageOf, tableRow } from "./trackTableIpc";

/**
 * Stands in for the Rust side of crates, for tests of adding tracks to them
 * (1cB-13, 1cB-14): a track table of `rows`, the crates and what is in them,
 * the commands called (`calls`, in order, with their arguments), and a
 * history of operations the top bar's Undo takes back one at a time.
 *
 * A row with `libraryTrackId: null` is a track outside the Library, which no
 * crate takes. The sibling-name rule is the real one in miniature: two things
 * in one folder can't share a name (letter case ignored).
 */
export function crateBackend(options: { rows: TableRow[]; crates: Crate[]; members?: Record<number, number[]> }) {
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
  };
  const called = (cmd: string) => state.calls.filter((call) => call.cmd === cmd);
  const refuse = (kind: string, params: Record<string, string> = {}) => {
    throw { kind, params };
  };
  const record = (kind: string, name: string, tracks: number, undo: () => void) => {
    const info: OperationInfo = { id: state.history.length + 1, kind, details: { name, from: null, tracks } };
    state.history.push({ info, undo });
    return info.id;
  };
  const recordingOf = (libraryTrackId: number) =>
    state.rows.find((row) => row.libraryTrackId === libraryTrackId)?.recordingId;

  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      state.calls.push({ cmd, args });
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
          const added: number[] = [];
          let alreadyThere = 0;
          let outsideLibrary = 0;
          for (const recording of new Set(args.recordings as number[])) {
            const row = state.rows.find((r) => r.recordingId === recording);
            if (row === undefined) return refuse("libraryTrackNotFound");
            if (row.libraryTrackId === null) outsideLibrary += 1;
            else if (held.has(recording)) alreadyThere += 1;
            else added.push(recording);
          }
          for (const recording of added) held.add(recording);
          state.members.set(id, held);
          const operationId =
            added.length === 0
              ? null
              : record("add_to_crate", target.name, added.length, () => {
                  for (const recording of added) held.delete(recording);
                });
          return { added: added.length, alreadyThere, outsideLibrary, operationId };
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
    },
    { shouldMockEvents: true },
  );
  return { state, called };
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
