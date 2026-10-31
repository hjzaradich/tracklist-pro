import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import type { TableFilter } from "../bindings";
import { doorWord } from "../library/doors";

/**
 * A door from an Overview number to the Library (1cC-4, "Every number is a
 * door"). With a `filter` it opens the Library with that filter chip on, so
 * the rows are what the number counted; with none it opens the whole Library
 * (the number of tracks is the Library itself).
 */
export function LibraryDoor({
  filter,
  className,
  name,
  children,
}: {
  filter?: TableFilter;
  className?: string;
  /** What a screen reader calls the link, when its text alone doesn't say. */
  name?: string;
  children: ReactNode;
}) {
  const word = filter === undefined ? null : doorWord(filter);
  return (
    <Link to="/library" search={word === null ? {} : { filter: word }} className={className} aria-label={name}>
      {children}
    </Link>
  );
}
