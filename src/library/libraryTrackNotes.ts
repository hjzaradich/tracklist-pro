import type { TFunction } from "i18next";
import type { LibraryTrack } from "../bindings";

/**
 * One thing worth saying about a Library track, before it's put into
 * words. New kinds of note (e.g. a fragile location, 1aD-7) join this
 * union, {@link trackNotes} and {@link describeNote}.
 */
export type TrackNote = { kind: "fileMissing" };

/** What to say about a Library track, most important first. Usually nothing. */
export function trackNotes(track: LibraryTrack): TrackNote[] {
  const notes: TrackNote[] = [];
  if (track.file === null || !track.file.present) notes.push({ kind: "fileMissing" });
  return notes;
}

/** A note in words. */
export function describeNote(note: TrackNote, t: TFunction<"library">): string {
  switch (note.kind) {
    case "fileMissing":
      return t("fileMissing");
  }
}
