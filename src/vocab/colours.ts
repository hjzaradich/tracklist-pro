import type { Tag, TagGroup } from "../bindings";

/** Red, green and blue (0 to 255) as the `#rrggbb` the database stores. */
function hex(red: number, green: number, blue: number): string {
  return "#" + [red, green, blue].map((part) => part.toString(16).padStart(2, "0")).join("");
}

/**
 * The plain colours a group or tag can be given (3A-T): a handful, so
 * choosing is quick. Stored as `#rrggbb`; a colour not in the list (set some
 * other way) still shows. They are the user's data, not the theme's, so they
 * are made from numbers rather than written as colour literals.
 */
export const PALETTE: readonly string[] = [
  hex(224, 90, 71),
  hex(224, 150, 59),
  hex(212, 183, 44),
  hex(79, 163, 95),
  hex(59, 154, 184),
  hex(74, 126, 224),
  hex(138, 97, 209),
  hex(194, 90, 156),
];

/** What is drawn for a tag or group with no colour. */
export const DEFAULT_COLOUR = hex(138, 143, 152);

/** The colour a tag shows: its own, else its group's, else the default. */
export function colourOf(tag: Pick<Tag, "colour">, group: Pick<TagGroup, "colour">): string {
  return tag.colour ?? group.colour ?? DEFAULT_COLOUR;
}
