import { mockIPC } from "@tauri-apps/api/mocks";
import type { Crate, Flow, GapSuggestion, OperationInfo, Playlist, TableQuery, TableRow, TargetPoint, Timeline } from "../bindings";
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
  /** Tracks outside the Library that "Add to Library" refuses (by recording id), with the reason. */
  refused?: Record<number, "noFile" | "fileMissing" | "matchNotConfirmed" | "lowBitrate">;
  /** How many entries of a rekordbox playlist have no Library track. */
  unmatched?: number;
  /** What "Fill the gap" offers after an entry (`null`: before the first). */
  gap?: (afterEntryId: number | null) => GapSuggestion[];
  /** Commands the backend refuses (the playlist is "not found"). */
  refusing?: string[];
}) {
  const state = {
    playlists: options.playlists.map((p) => ({ ...p, entries: [...p.entries] })),
    crates: [...(options.crates ?? [])],
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
    history: [] as { info: OperationInfo; undo: () => void }[],
    nextId: 100,
    /** Each playlist's target shape (3C-P2b). */
    targets: {} as Record<number, TargetPoint[]>,
  };
  const called = (cmd: string) => state.calls.filter((call) => call.cmd === cmd);
  const refuse = (kind: string, params: Record<string, string> = {}): never => {
    throw { kind, params };
  };
  const record = (
    kind: string,
    name: string,
    tracks: number | null,
    undo: () => void,
    from: string | null = null,
    newToLibrary: number | null = null,
  ) => {
    const info: OperationInfo = {
      id: state.history.length + 1,
      kind,
      details: { name, from, tracks, newToLibrary, newCrate: null },
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
  /**
   * A playlist sorted by fit after a track: every entry of that track first,
   * then the others in reverse order (the real order is tested in Rust), each
   * row with a fit.
   */
  const byFit = (id: number, after: number): TableRow[] => {
    const all = tableOf(id);
    const first = all.filter((row) => row.recordingId === after);
    const rest = all.filter((row) => row.recordingId !== after).reverse();
    return [...first, ...rest].map((row) => ({ ...row, fit: row.recordingId === after ? { dots: 0, reason: null } : { dots: 2, reason: null } }));
  };
  const listed = (source: TableQuery["source"]): TableRow[] =>
    source.kind === "playlist"
      ? tableOf(source.id)
      : source.kind === "playlistByFit"
        ? byFit(source.id, source.after)
        : rows;
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

  /** The target at `at` (0 to 1): a straight line between the points, flat beyond the ends; none without points. */
  const targetAt = (points: TargetPoint[], at: number): number | null => {
    const known = points.flatMap((p) => (p.at === null || p.energy === null ? [] : [{ at: p.at, energy: p.energy }]));
    const first = known[0];
    const last = known.at(-1);
    if (first === undefined || last === undefined) return null;
    if (at <= first.at) return first.energy;
    if (at >= last.at) return last.energy;
    const right = known.findIndex((p) => p.at >= at);
    const [a, b] = [known[right - 1], known[right]] as [(typeof known)[number], (typeof known)[number]];
    return b.at === a.at ? b.energy : a.energy + ((b.energy - a.energy) * (at - a.at)) / (b.at - a.at);
  };

  const handle = (cmd: string, args: Record<string, unknown>): unknown => {
    if (options.refusing?.includes(cmd)) return refuse("playlistNotFound");
    switch (cmd) {
      case "list_playlists":
        return state.playlists.map(({ entries, ...p }) => ({ ...p, trackCount: entries.length }));
      case "list_crates":
        return state.crates.map((c) => ({ ...c, trackCount: 0 }));
      case "track_table_page": {
        const query = args.query as TableQuery;
        const source = query.source;
        return pageOf(listed(source), query);
      }
      case "track_table_ids": {
        const query = args.query as TableQuery;
        const all = listed(query.source);
        return all.slice(query.offset, query.offset + query.limit).map((row) => row.recordingId);
      }
      case "smart_crate_rule":
        return { combine: "all", conditions: [] };
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
        const leftOut = { noFile: 0, fileMissing: 0, matchNotConfirmed: 0, lowBitrate: 0 };
        const joining: number[] = [];
        const linked: TableRow[] = [];
        for (const recording of asked) {
          const row = rowOf(recording);
          if (row === undefined) return refuse("libraryTrackNotFound");
          if (row.libraryTrackId === null) {
            const why = options.refused?.[recording];
            if (why !== undefined) {
              leftOut[why] += 1;
              continue;
            }
            linked.push(row);
          }
          joining.push(recording);
        }
        // As in a crate: the tracks outside the Library join it in the same operation.
        for (const row of linked) {
          row.libraryTrackId = state.nextId++;
          row.inLibrary = true;
        }
        const added = newEntries(joining);
        const before = [...target.entries];
        target.entries.push(...added);
        const undo = () => {
          target.entries = before;
          for (const row of linked) {
            row.libraryTrackId = null;
            row.inLibrary = false;
          }
        };
        const operationId =
          added.length === 0 ? null : record("add_to_playlist", target.name, added.length, undo, null, linked.length);
        return { added: added.length, alsoAddedToLibrary: linked.length, leftOut, operationId };
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
      case "set_playlist_order": {
        const target = playlist(args.id);
        const named = [...new Set(args.entries as number[])].flatMap((entry) => {
          const found = target.entries.find(([id]) => id === entry);
          return found === undefined ? [] : [found];
        });
        const before = [...target.entries];
        const after = [...named, ...before.filter(([entry]) => !named.some(([id]) => id === entry))];
        if (after.every((entry, i) => entry === before[i])) return { changed: named.length, skipped: 0, operationId: null };
        target.entries = after;
        const operationId = record("move_in_playlist", target.name, named.length, () => (target.entries = before));
        return { changed: named.length, skipped: 0, operationId };
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
      case "playlist_timeline": {
        const target = playlist(args.id);
        const points = state.targets[target.id] ?? [];
        const rowsOf = tableOf(target.id);
        let startMs = 0;
        const entries = rowsOf.map((row, position) => {
          const at = rowsOf.length <= 1 ? 0.5 : position / (rowsOf.length - 1);
          const wanted = targetAt(points, at);
          const entry = {
            entryId: row.entryId ?? 0,
            position,
            recordingId: row.recordingId,
            title: row.title,
            artist: row.artist,
            durationMs: row.timeMs,
            bpm: row.bpm,
            key: row.key,
            keyEstimated: row.keyEstimated,
            energy: row.energy,
            startMs,
            target: wanted,
            gap: wanted === null || row.energy === null ? null : row.energy - wanted,
          };
          startMs += row.timeMs ?? 0;
          return entry;
        });
        // A flag is unknown when either track lacks the value, a clash when the values differ much.
        const flow = (a: number | string | null, b: number | string | null, far: (a: never, b: never) => boolean): Flow =>
          a === null || b === null ? "unknown" : far(a as never, b as never) ? "clash" : "fine";
        const transitions = rowsOf.slice(1).map((next, i) => {
          const row = rowsOf[i] as TableRow;
          return {
            fromEntry: row.entryId ?? 0,
            toEntry: next.entryId ?? 0,
            key: flow(row.key, next.key, (a, b) => a !== b),
            bpm: flow(row.bpm, next.bpm, (a: number, b: number) => Math.abs(b - a) > 6),
            bpmDelta: row.bpm === null || next.bpm === null ? null : next.bpm - row.bpm,
            energy: flow(row.energy, next.energy, (a: number, b: number) => Math.abs(b - a) >= 3),
            energyStep: row.energy === null || next.energy === null ? null : next.energy - row.energy,
          };
        });
        const timeline: Timeline = {
          playlist: { ...target, trackCount: target.entries.length },
          entries,
          transitions,
          totalMs: startMs,
          unknownDurations: rowsOf.filter((row) => row.timeMs === null).length,
          target: points,
          tolerance: "tight",
        };
        return timeline;
      }
      case "set_playlist_target": {
        const target = playlist(args.id);
        const before = state.targets[target.id];
        state.targets[target.id] = [...(args.points as TargetPoint[])].sort((a, b) => (a.at ?? 0) - (b.at ?? 0));
        return record("set_playlist_target", target.name, null, () => {
          if (before === undefined) delete state.targets[target.id];
          else state.targets[target.id] = before;
        });
      }
      case "clear_playlist_target": {
        const target = playlist(args.id);
        const before = state.targets[target.id];
        if (before === undefined) return null;
        delete state.targets[target.id];
        return record("clear_playlist_target", target.name, null, () => (state.targets[target.id] = before));
      }
      case "fill_gap":
        return { suggestions: options.gap?.(args.afterEntryId as number | null) ?? [], considered: rows.length, target: null };
      case "add_tracks_to_playlist": {
        const target = playlist(args.id);
        const recordings = (args.tracks as number[]).flatMap((track) => rows.find((r) => r.libraryTrackId === track)?.recordingId ?? []);
        const before = [...target.entries];
        const at = Math.min((args.at as number | null) ?? before.length, before.length);
        target.entries = [...before.slice(0, at), ...newEntries(recordings), ...before.slice(at)];
        const operationId = record("add_to_playlist", target.name, recordings.length, () => (target.entries = before));
        return { changed: recordings.length, skipped: 0, operationId };
      }
      case "next_undo_operation":
        return { operation: state.history.at(-1)?.info ?? null, refusal: null };
      case "undo_last_operation": {
        const last = state.history.pop();
        if (last === undefined) return { status: "nothingToUndo" };
        last.undo();
        return { status: "undone", operation: last.info };
      }
      case "remove_library_track": {
        const row = rows.find((r) => r.libraryTrackId === args.id);
        const before = state.playlists.map((p) => [p, p.entries] as const);
        for (const [p, entries] of before) p.entries = entries.filter(([, recording]) => recording !== row?.recordingId);
        return record("remove_from_library", "", null, () => {
          for (const [p, entries] of before) p.entries = entries;
        });
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
