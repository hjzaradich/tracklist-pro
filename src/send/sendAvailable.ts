/**
 * Whether "Send to rekordbox" can be opened (1cB-11): the Library has been
 * read and holds tracks. The Overview's button, the command palette's entry
 * and the palette's request all ask this one function, so none is open while
 * another is greyed out. (With no tracks the Overview shows the first-run
 * flow, with no Send; with the Library unreadable the button is greyed out
 * and says why.) Opening it never sends anything.
 */
export function sendAvailable(libraryTracks: number | undefined): boolean {
  return libraryTracks !== undefined && libraryTracks > 0;
}
