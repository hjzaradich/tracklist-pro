import type { LibraryTrack, TableRow } from "../bindings";

/**
 * What the Library's row actions (Add to crate, Remove) need of a track.
 * A Library track has all of it, and so does a row of the track table that
 * is a Library track ({@link libraryRow}).
 */
export type LibraryRow = Pick<LibraryTrack, "id" | "title" | "file" | "openConflicts">;

/** The row's Library track, or `null` for a track that isn't in the Library. */
export function libraryRow(row: TableRow): LibraryRow | null {
  return row.libraryTrackId === null
    ? null
    : { id: row.libraryTrackId, title: row.title, file: row.file, openConflicts: row.openConflicts };
}
