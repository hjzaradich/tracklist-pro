import { mockIPC } from "@tauri-apps/api/mocks";
import type { Crate, OperationInfo, Playlist, TableQuery, TableRow } from "../bindings";
import { quietAnswer } from "./quietCommands";
import { pageOf } from "./trackTableIpc";

/**
 * Stands in for the Rust side of playlists (3C-P1b): a track table of `rows`
 * (the Library, rows with a `libraryTrackId`), the playlists with their
 * entries in order (a track may be in one twice), the commands called
 * (`calls`, in order, with their arguments) and a history of operations the
 * top bar's Undo takes back one at a time. A playlist's table pages come as
 * the real ones do: one row per entry, carrying its `entryId`.
 */
export interface FakePlaylist extends Omit<Playlist, "trackCount"> {
  /** The entries in order: [entry id, recording id]. */
  entries: [number, number][];
}

export function playlistBackend(options: {
  rows: TableRow[];
  playlists: FakePlaylist[];
  crates?: Crate[];
  /** What "Make an app playlist from this rekordbox playlist" offers. */
  rekordbox?: { path: string[]; entries: number }[];
  /** How many entries of a rekordbox playlist have no Library track. */
  unmatched?: number;
}) {
  const state = {
    playlists: options.playlists.map((p) => ({ ...p, entries: [...p.entries] })),
    crates: [...(options.crates ?? [])],
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
    history: [] as { info: OperationInfo; undo: () => void }[],
    nextId: 100,
  };
  const called = (cmd: string) => state.calls.filter((call) => call.cmd === cmd);
  const refuse = (kind: string, params: Record<string, string> = {}): never => {
    throw { kind, params };
  };
  const record = (kind: string, name: string, tracks: number | null, undo: () => void, from: string | null = null) => {
    const info: OperationInfo = {
      id: state.history.length + 1,
      kind,
      details: { name, from, tracks, newToLibrary: null, newCrate: null },
    };
    state.history.push({ info, undo });
    return info.id;
  };
  const playlist = (id: unknown) => state.playlists.find((p) => p.id === id) ?? refuse("playlistNotFound");
  const rows = options.rows;
  const rowOf = (recording: number) => rows.find((r) => r.recordingId === recording);
  /** The entries of a playlist as table rows. */
  const tableOf = (id: number): TableRow[] =>
    playlist(id).entries.flatMap(([entryId, recording]) => {
      const row = rowOf(recording);
      return row === undefined ? [] : [{ ...row, entryId }];
    });
  const unique = (name: string, except?: number) => {
    const taken = state.playlists.find((p) => p.id !== except && p.name.toLowerCase() === name.toLowerCase());
    if (name === "") refuse("playlistNameEmpty");
    if (taken) refuse("playlistNameTaken", { name: taken.name });
  };
  const make = (name: string, entries: [number, number][], origin: "app" | "rekordbox" = "app", sourcePath: string[] | null = null) => {
    const id = state.nextId++;
    state.playlists.push({ id, name, origin, sourcePath, entries });
    return id;
  };
  const newEntries = (recordings: number[]): [number, number][] => recordings.map((r) => [state.nextId++, r]);

  const handle = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "list_playlists":
        return state.playlists.map(({ entries, ...p }) => ({ ...p, trackCount: entries.length }));
      case "list_crates":
        return state.crates.map((c) => ({ ...c, trackCount: 0 }));
      case "track_table_page": {
        const query = args.query as TableQuery;
        const source = query.source;
        return pageOf(source.kind === "playlist" ? tableOf(source.id) : rows, query);
      }
      case "track_table_ids": {
        const query = args.query as TableQuery;
        const all = query.source.kind === "playlist" ? tableOf(query.source.id) : rows;
        return all.slice(query.offset, query.offset + query.limit).map((row) => row.recordingId);
      }
      case "track_table_position":
        return null;
      case "create_playlist": {
        const name = String(args.name).trim();
        unique(name);
        const id = make(name, []);
        record("create_playlist", name, null, () => {
          state.playlists = state.playlists.filter((p) => p.id !== id);
        });
        return id;
      }
      case "rename_playlist": {
        const target = playlist(args.id);
        const name = String(args.name).trim();
        if (name === target.name) return null;
        unique(name, target.id);
        const before = target.name;
        target.name = name;
        return record("rename_playlist", name, null, () => (target.name = before), before);
      }
      case "delete_playlist": {
        const target = playlist(args.id);
        state.playlists = state.playlists.filter((p) => p !== target);
        return record("delete_playlist", target.name, target.entries.length, () => state.playlists.push(target));
      }
      case "add_recordings_to_playlist": {
        const target = playlist(args.id);
        const asked = [...new Set(args.recordings as number[])];
        const inLibrary = asked.filter((r) => rowOf(r)?.libraryTrackId != null);
        const added = newEntries(inLibrary);
        const before = [...target.entries];
        target.entries.push(...added);
        const operationId =
          added.length === 0 ? null : record("add_to_playlist", target.name, added.length, () => (target.entries = before));
        return { changed: added.length, skipped: asked.length - added.length, operationId };
      }
      case "remove_from_playlist": {
        const target = playlist(args.id);
        const gone = new Set(args.entries as number[]);
        const before = [...target.entries];
        target.entries = target.entries.filter(([entry]) => !gone.has(entry));
        const changed = before.length - target.entries.length;
        const operationId =
          changed === 0 ? null : record("remove_from_playlist", target.name, changed, () => (target.entries = before));
        return { changed, skipped: gone.size - changed, operationId };
      }
      case "move_in_playlist": {
        const target = playlist(args.id);
        const moving = new Set(args.entries as number[]);
        const before = [...target.entries];
        const carried = before.filter(([entry]) => moving.has(entry));
        const rest = before.filter(([entry]) => !moving.has(entry));
        const stay = args.before as number | null;
        // As the real one: a `before` that is itself moved means the first entry after it that is not.
        const from = stay === null ? -1 : before.findIndex(([entry]) => entry === stay);
        if (stay !== null && from < 0) return refuse("playlistEntryNotFound");
        const target_ = from < 0 ? undefined : before.slice(from).find(([entry]) => !moving.has(entry));
        const at = target_ === undefined ? rest.length : rest.findIndex(([entry]) => entry === target_[0]);
        const after = [...rest.slice(0, at), ...carried, ...rest.slice(at)];
        const same = after.every((entry, i) => entry === before[i]);
        if (same) return { changed: carried.length, skipped: 0, operationId: null };
        target.entries = after;
        const operationId = record("move_in_playlist", target.name, carried.length, () => (target.entries = before));
        return { changed: carried.length, skipped: 0, operationId };
      }
      case "save_playlist_as_crate": {
        const source = playlist(args.id);
        const name = String(args.name).trim();
        const id = state.nextId++;
        state.crates.push({ id, parentId: null, kind: "static", name, trackCount: 0 });
        return record("save_playlist_as_crate", name, source.entries.length, () => {
          state.crates = state.crates.filter((c) => c.id !== id);
        });
      }
      case "playlist_from_crate": {
        const name = String(args.name).trim();
        unique(name);
        const order = args.order as number[];
        const recordings = order.flatMap((track) => rows.find((r) => r.libraryTrackId === track)?.recordingId ?? []);
        const id = make(name, newEntries(recordings));
        record("playlist_from_crate", name, recordings.length, () => {
          state.playlists = state.playlists.filter((p) => p.id !== id);
        });
        return id;
      }
      case "rekordbox_playlists_to_copy":
        return options.rekordbox ?? [];
      case "playlist_from_rekordbox": {
        const name = String(args.name).trim();
        unique(name);
        const path = args.path as string[];
        const found = (options.rekordbox ?? []).find((p) => p.path.join("/") === path.join("/"));
        if (found === undefined) return refuse("rekordboxPlaylistNotFound");
        const kept = Math.max(0, Math.min(found.entries - (options.unmatched ?? 0), rows.length));
        const id = make(name, newEntries(rows.slice(0, kept).map((r) => r.recordingId)), "rekordbox", path);
        record("playlist_from_rekordbox", name, kept, () => {
          state.playlists = state.playlists.filter((p) => p.id !== id);
        });
        return { playlistId: id, added: kept, notInLibrary: found.entries - kept };
      }
      case "next_undo_operation":
        return { operation: state.history.at(-1)?.info ?? null, refusal: null };
      case "undo_last_operation": {
        const last = state.history.pop();
        if (last === undefined) return { status: "nothingToUndo" };
        last.undo();
        return { status: "undone", operation: last.info };
      }
      case "crates_of_track":
        return [];
      case "track_tag_sources":
        return { inRekordbox: false, fields: [], confirmedEntry: null };
      default:
        return quietAnswer(cmd);
    }
  };

  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      state.calls.push({ cmd, args });
      return handle(cmd, args);
    },
    { shouldMockEvents: true },
  );
  return { state, called };
}
