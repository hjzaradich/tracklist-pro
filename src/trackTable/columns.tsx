import type { TFunction } from "i18next";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { SortColumn, TableRow } from "../bindings";
import { formatKey, type KeyNotation } from "../keys/keyNotation";
import { describeNote, trackNotes } from "../library/libraryTrackNotes";
import { PlayButton } from "../player/PlayButton";
import { formatTime } from "../player/formatTime";
import styles from "./TrackTable.module.css";

/**
 * The columns of the track table (ROADMAP 1.11 "The track table").
 *
 * The list of columns is data, not markup: {@link COLUMNS} says what each
 * column is, and {@link DEFAULT_COLUMNS} which columns each screen shows, in
 * order. The table is given a list of {@link ColumnId}s and draws exactly
 * those, so the column editor (1cA-5) only has to hand it another list (and,
 * later, widths).
 *
 * Left out for now, because the app has no data for them yet: Album, Year,
 * Energy, Comment, Label, Play count, Last played, Colour and the other
 * columns a DJ library has. Each joins by a line here and a field of
 * `TableRow`.
 */
export type ColumnId =
  | "title"
  | "artist"
  | "bpm"
  | "key"
  | "time"
  | "genre"
  | "rating"
  | "dateAdded"
  | "file"
  | "folder";

/** What a cell needs to be worked out. */
export interface CellContext {
  t: TFunction<"trackTable">;
  keyNotation: KeyNotation;
  language: string;
}

export interface ColumnDef {
  id: ColumnId;
  /** What the backend sorts by when this heading is clicked. */
  sort: SortColumn;
  /** The width in pixels a column has, and the least it shrinks to. */
  width: number;
  /** The share of the width left over that the column takes, if any. */
  grow?: number;
  /** Numbers: right-aligned, in tabular figures. */
  numeric?: boolean;
  /** The plain text the cell shows (the cell's hover text when it is cut off). */
  text: (row: TableRow, ctx: CellContext) => string;
  /** The cell, when it is more than its text. */
  cell?: (row: TableRow, ctx: CellContext) => ReactNode;
}

/** A BPM with one decimal: `124.0`. */
export function formatBpm(bpm: number | null): string {
  return bpm === null ? "" : bpm.toFixed(1);
}

/** A length in milliseconds as `m:ss`. */
export function formatLength(ms: number | null): string {
  return ms === null ? "" : formatTime(Math.round(ms / 1000));
}

/** A date from a UTC timestamp, in the user's own time zone and language. */
export function formatDate(timestamp: string | null, language: string): string {
  if (timestamp === null) return "";
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? "" : date.toLocaleDateString(language);
}

/** The folder of a file path (`E:\Music\a.mp3` is `E:\Music`). */
export function folderOf(path: string, name: string): string {
  const folder = path.endsWith(name) ? path.slice(0, path.length - name.length) : path;
  return folder.replace(/[\\/]+$/, "");
}

const STARS = 5;

/** The title the table shows for a track: its title, or its file's name. */
export function shownName(row: Pick<TableRow, "title" | "file">): string {
  return row.title ?? row.file?.name ?? "";
}

/** The title cell: a play button for the track's file, its title and notes about it. */
function TitleCell({ row }: { row: TableRow }) {
  const { t } = useTranslation("library");
  const name = shownName(row);
  const notes = trackNotes(row);
  return (
    <>
      {row.file !== null && (
        <span className={styles.play}>
          <PlayButton file={{ id: row.file.id, name }} />
        </span>
      )}
      <span className={styles.text} title={name}>
        {name}
      </span>
      {notes.length > 0 && (
        <span className={styles.notes}>
          {notes.map((note) => (
            <span
              key={note.kind}
              className={styles.note}
              data-note={note.kind}
              title={describeNote(note, t)}
            >
              {describeNote(note, t)}
            </span>
          ))}
        </span>
      )}
    </>
  );
}

function RatingCell({ stars, t }: { stars: number; t: TFunction<"trackTable"> }) {
  return (
    <span role="img" aria-label={t("rating", { count: stars })} title={t("rating", { count: stars })}>
      {"\u2605".repeat(stars)}
      {"\u2606".repeat(STARS - stars)}
    </span>
  );
}

export const COLUMNS: Readonly<Record<ColumnId, ColumnDef>> = {
  title: {
    id: "title",
    sort: "title",
    width: 240,
    grow: 3,
    text: shownName,
    cell: (row) => <TitleCell row={row} />,
  },
  artist: {
    id: "artist",
    sort: "artist",
    width: 160,
    grow: 2,
    text: (row) => row.artist ?? "",
  },
  bpm: {
    id: "bpm",
    sort: "bpm",
    width: 64,
    numeric: true,
    text: (row) => formatBpm(row.bpm),
  },
  key: {
    id: "key",
    sort: "key",
    width: 56,
    numeric: true,
    text: (row, ctx) => formatKey(row.key, ctx.keyNotation) ?? "",
  },
  time: {
    id: "time",
    sort: "time",
    width: 64,
    numeric: true,
    text: (row) => formatLength(row.timeMs),
  },
  genre: {
    id: "genre",
    sort: "genre",
    width: 120,
    grow: 1,
    text: (row) => row.genre ?? "",
  },
  rating: {
    id: "rating",
    sort: "rating",
    width: 88,
    text: (row, ctx) => (row.rating ? ctx.t("rating", { count: row.rating }) : ""),
    cell: (row, ctx) => (row.rating ? <RatingCell stars={row.rating} t={ctx.t} /> : null),
  },
  dateAdded: {
    id: "dateAdded",
    sort: "dateAdded",
    width: 104,
    numeric: true,
    text: (row, ctx) => formatDate(row.addedAt, ctx.language),
  },
  file: {
    id: "file",
    sort: "file",
    width: 160,
    grow: 2,
    text: (row) => row.file?.name ?? "",
  },
  folder: {
    id: "folder",
    sort: "folder",
    width: 200,
    grow: 2,
    text: (row) => (row.file ? folderOf(row.file.path, row.file.name) : ""),
  },
};

/** The screens that show the table. */
export type TableScreen = "library" | "allMusic" | "crate";

const LIBRARY_COLUMNS: readonly ColumnId[] = [
  "title",
  "artist",
  "bpm",
  "key",
  "time",
  "genre",
  "rating",
  "dateAdded",
];

/**
 * The columns each screen shows, in order (owner decision, 2026-10-26):
 * All music adds the file and its folder; a crate is the Library's.
 */
export const DEFAULT_COLUMNS: Readonly<Record<TableScreen, readonly ColumnId[]>> = {
  library: LIBRARY_COLUMNS,
  allMusic: [...LIBRARY_COLUMNS, "file", "folder"],
  crate: LIBRARY_COLUMNS,
};

/** The columns a screen shows now. The column editor (1cA-5) will make this a setting. */
export function useColumns(screen: TableScreen): readonly ColumnId[] {
  return DEFAULT_COLUMNS[screen];
}
