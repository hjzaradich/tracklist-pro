# Design style

How tracklist-pro looks, and what the owner has decided about it. The designer session ("TLP · Designer") owns this file, `src/theme/tokens.css` and every `*.module.css`. What a screen shows, the order of its controls and its behavior are not design's to change (they go to the foreman); wording goes to the copy editor ([copy-style.md](copy-style.md)).

## Rules that always hold

- **Colors only through theme tokens.** No color literal outside `src/theme/tokens.css`. A test checks it.
- **Two themes, dark (default) and light.** Both define the same token names. Check every change in both.
- **One accent (amber), meaning "selected / active".** Warning is yellow, never the accent's hue; danger is red; success is green. Color only where it means something.
- **No pure black, and no pure-white text** (the tests check both).
- **Never color alone to carry meaning.** Pair it with a word, a shape or a position.
- **A visible keyboard focus** (`:focus-visible` ring, `--color-focus-ring`) on everything that takes focus.
- **No motion that can't be turned off.** Honor "reduce motion".
- **Change a shared value before adding a one-off override**, so the app stays consistent.
- **Offline and nothing sent:** no font or asset from the network, no new dependency without the foreman's yes (licenses must be GPL-compatible).
- **Sample data only** in anything committed: no real track or artist names, no screenshot of a real library.

## The tokens today

All in `src/theme/tokens.css`. Components use the variables only.

### Shared (both themes)

| Token | Value | Used for |
|---|---|---|
| `--font-sans` | Segoe UI Variable Text, Segoe UI, system-ui | All text |
| `--font-mono` | Cascadia Mono, Consolas | Paths, ids |
| `--font-size-sm` | 12px | Secondary detail |
| `--font-size-md` | 14px | Body (the page default) |
| `--font-size-lg` | 16px | Headings |
| `--font-size-xl` | 20px | Screen titles |
| `--space-1` … `--space-6` | 4, 8, 12, 16, 24px | Gaps and padding |
| `--radius-sm`, `--radius-md` | 3px, 6px | Corners |
| `--row-height` | 28px | List rows (compact density, ROADMAP 1.1) |
| `--motion-fast` | 80ms (0 with Windows' "reduce motion") | The hover fade |

### Colors

| Token | Dark | Light | Used for |
|---|---|---|---|
| `--color-bg` | `#151719` | `#e9ebed` | Page |
| `--color-surface` | `#1c1f22` | `#f4f5f6` | Panels |
| `--color-surface-raised` | `#24282c` | `#fbfbfc` | Raised panels, menus |
| `--color-surface-hover` | `#2c3035` | `#d8dce0` | Hover |
| `--color-row-stripe` | `#1b1e21` | `#f2f4f5` | Every other row of a long list |
| `--color-selected` | `#3d301b` | `#f0dcb6` | A selected row, crate or stage |
| `--color-border` | `#343a40` | `#cdd2d7` | Dividers |
| `--color-border-strong` | `#4a5158` | `#aab1b8` | Controls |
| `--color-text` | `#e3e5e8` | `#1f2326` | Text |
| `--color-text-muted` | `#9ba2aa` | `#59616a` | Secondary text |
| `--color-text-disabled` | `#646b73` | `#969da5` | Unavailable |
| `--color-accent` | `#e0a13c` | `#a86a12` | Selected, active, the main action (bar, border, fill) |
| `--color-accent-hover` | `#ebb259` | `#8f5a0e` | Accent hover |
| `--color-on-accent` | `#1c1f22` | `#fbfbfc` | Text on the accent |
| `--color-focus-ring` | `#e0a13c` | `#a86a12` | Keyboard focus |
| `--color-danger` | `#e5675c` | `#b83a30` | Data loss, errors |
| `--color-warning` | `#e6dc70` | `#625e00` | Warnings |
| `--color-success` | `#5fb87a` | `#2f7d4a` | Done, ok |

## Seeing the app: the preview

`npm run design` starts a development copy of the app on made-up sample data in `C:\dev\tracklist-pro-preview` (its own data folder, its own rekordbox "Documents", its own browser profile). The window title says "(preview)". The real app data folder is never opened. The first start builds the app and makes the sample (a few minutes); after that it redraws as a style file is saved. `npm run design:reset` deletes the sample; the next start makes it again.

The sample has about 250 tracks: long titles, untagged files, duplicates, versions, missing files, an empty crate, a rekordbox export with analysis, and a written send. Activity is empty on purpose, so the empty states can be seen.

## The owner's decisions

One line each: the date, what was decided, and why, so the next change holds to it. "Style" decisions are the designer's to make in CSS; "Foreman" ones need a structure or behavior change and go to the foreman first.

<!-- Format: - YYYY-MM-DD · Decision · Why · Style or Foreman -->

- 2026-10-10 · **Density: compact rows stay the default** (28px). Text gets a little bigger instead of the rows getting taller. · A library is scanned, not read; more rows on screen is the point. · Style
- 2026-10-10 · **Text size: 14px body, 12px secondary detail** (up from 13px / 12px). · Easier to read in a dim room and at arm's length. · Style
- 2026-10-10 · **Long lists get faint row stripes, and a clearly stronger "selected" look.** · Lists run to hundreds of rows; stripes keep the eye on one row, and "selected" must beat "hover" and "stripe". · Style
- 2026-10-10 · **The amber accent means "selected or active", and nothing else.** Warnings stay yellow. · Amber on screen then always tells you where you are. · Style
- 2026-10-10 · **Dark stays the default.** A "follow Windows" option may come later. · Dark suits a dim room. The option is a new setting. · Foreman (backlog)
- 2026-10-10 · **Long titles are cut off with "…" so a row stays one line.** The full text shows on hover, and in the Details panel once that panel is built. · A tidy grid beats a ragged one. · Style (cut-off), hover text done as `title` (below), Foreman (Details panel, waits for the Phase 1a checkpoint)
- 2026-10-10 · **One filled accent button per screen, everything else quiet.** Each screen is walked through with the owner to name its one thing. · The loudest thing on a screen should be the thing to do next. · Style, with Foreman for any screen that needs a control moved
- 2026-10-10 · **Almost no motion:** a quick fade on hover and focus only, and none when Windows' "reduce motion" is on. · Motion costs attention, and some people can't have it. · Style

## The one exception to "class names only"

A cell that is cut off with "…" gets a `title` attribute (the native hover text) set to **the same value the cell shows**: no new text, no new data, no locale key. Allowed by the foreman, 2026-10-10. A test per list holds it ("a cut-off title cell carries its full text").

Hover text does not reach keyboard users, and a touch screen has no hover. So the full text must also live in the Details panel when that panel is built (today it is a placeholder; showing a track's title and path there is a new feature, logged by the foreman for the Phase 1a checkpoint). Until then the full title is only on hover.

## How the decisions look in the styles

- **Text:** `--font-size-md` 14px; `--font-size-sm` 12px. A test holds those floors.
- **Stripes and selected:** three steps, each a clear step stronger than the last: a stripe (faintest), hover, selected (strongest). A test holds the order in both themes, and that text stays readable on each.
- **Selected** is the warm tint `--color-selected` on the sidebar's active stage and the chosen crate, with the accent bar or border kept.
- **Cut-off:** Library and All music keep a row to one line (`table-layout: fixed`, title 28% and artist 18% of the width, a fixed actions column); a long title, artist or path ends in "…". Rows are at the compact row height; a row with a note under its path is taller on purpose.
- **Motion:** one hover fade (`--motion-fast`), colors only, set on hover so a theme switch stays instant. No stylesheet may animate; tests check it.

## Open questions for the owner

None yet.
