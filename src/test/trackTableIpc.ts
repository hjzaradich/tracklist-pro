import type { LibraryTrack, TablePage, TableQuery, TableRow } from "../bindings";

/** A row of the track table, with made-up values: override what a test is about. */
export function tableRow(id: number, fields: Partial<TableRow> = {}): TableRow {
  return {
    recordingId: id + 100,
    libraryTrackId: id,
    title: `Synthetic Tune ${id}`,
    artist: `Made Up Artist ${id}`,
    bpm: null,
    key: null,
    timeMs: null,
    genre: null,
    rating: null,
    addedAt: "2026-09-30T10:00:00.000Z",
    file: {
      id,
      path: String.raw`E:\Music\tune ` + id + ".mp3",
      name: `tune ${id}.mp3`,
      present: true,
      driveConnected: true,
    },
    inLibrary: true,
    matchNotConfirmed: false,
    fragile: null,
    sourceMissing: false,
    openConflicts: 0,
    betterFile: null,
    ...fields,
  };
}

/** The row for a Library track, as the table shows it. */
export function rowOfLibraryTrack(track: LibraryTrack): TableRow {
  return tableRow(track.id, {
    recordingId: track.recordingId,
    libraryTrackId: track.id,
    title: track.title,
    artist: track.artist,
    file: track.file,
    fragile: track.fragile,
    sourceMissing: track.sourceMissing,
    openConflicts: track.openConflicts,
    addedAt: track.addedAt,
    betterFile: track.betterFile,
  });
}

/**
 * What the backend answers a window with, for a list of `rows` already in
 * the order the test wants: the window asked for, and the count when asked.
 */
export function pageOf(rows: readonly TableRow[], query: TableQuery): TablePage {
  return {
    total: query.withTotal ? rows.length : null,
    offset: query.offset,
    rows: rows.slice(query.offset, query.offset + query.limit),
    behind: false,
  };
}

/** The query inside the arguments of a `track_table_page` call. */
export function queryOf(args: unknown): TableQuery {
  return (args as { query: TableQuery }).query;
}
