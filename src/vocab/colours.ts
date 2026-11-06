import type { Tag, TagGroup } from "../bindings";

/**
 * The plain colours a group or tag can be given (3A-T): a handful, so
 * choosing is quick. Stored as `#rrggbb`; a colour not in the list (set some
 * other way) still shows.
 */
export const PALETTE = ["#e05a47", "#e0963b", "#d4b72c", "#4fa35f", "#3b9ab8", "#4a7ee0", "#8a61d1", "#c25a9c"] as const;

/** What is drawn for a tag or group with no colour. */
export const DEFAULT_COLOUR = "#8a8f98";

/** The colour a tag shows: its own, else its group's, else the default. */
export function colourOf(tag: Pick<Tag, "colour">, group: Pick<TagGroup, "colour">): string {
  return tag.colour ?? group.colour ?? DEFAULT_COLOUR;
}
