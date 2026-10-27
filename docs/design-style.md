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

## Review screen (decided 2026-10-13)

- 2026-10-13 · **Two panes: the queue at a fixed ~20rem, the comparison table takes the rest.** The file column is a share of the table (never under 10rem) and long names end in "…" with the path on hover. · The table has 7-8 columns and is what is read; the queue is short names. · Style
- 2026-10-13 · **Below 1100px the queue goes on top (shorter) and the table is full width.** · No column is scrolled out of sight while comparing; the table fits without sideways scrolling at the 900px minimum. · Style
- 2026-10-13 · **"Suggested" and "Linked" are words in neutral bordered labels** ("Suggested" bold and filled, "Linked" an outline). The accent is not used for them. · Never colour alone, and amber means selected or active. · Style
- 2026-10-13 · **Actions: one row directly under the comparison table, acting on the suggested file.** The one filled accent button is "keep the suggested file", the rest are quiet, a small line under them (`.keys`) shows the keys (Enter, number keys, S). A file-specific action goes in a quiet cell at the end of its row (`.rowAction`). · The loudest thing is the thing to do next. · Style, with Foreman for the buttons themselves (1bE)

## Review fits the window (1bF-4, DOGFOOD B-6)

Owner-approved on 2026-10-24, after a real Library showed three nested scrollbars, a queue of about four visible items and a comparison whose actions fell out of view. The rules, at about 1270x820:

- **No page scroll.** Every region scrolls on its own, once, and the screen itself never does.
- **Three regions:** the queue (left, a fixed 15-16rem, compact items), the comparison (right) and, when opened, Missing. The comparison's table scrolls inside it (its header stays put); its heading, suggestion and doubt stay above it, and **the actions stay docked under it**, always in view.
- **Missing is one line** (its heading and how many tracks) until it is opened. Open, it takes at most 45% of the screen and scrolls inside. It opens by itself when nothing is missing or when something went wrong, so a message is never hidden.
- **The Track column keeps its room** (never under 8rem), the facts columns are secondary text (12px), and the quality verdict may wrap: the other columns give way before the track and the file.
- **The stage gutter is 16px** (it was 24px), which every screen shares.
- **Below 1100px** the queue goes on top, shorter (at most 14rem or 30% of the height), and the comparison takes the full width.

## The track table, its controls and the crate tree (1cA style pass)

- **Controls above the table are as tall as a row** (28px, `--row-height`), and never under 24px, the smallest target WCAG 2.2 allows: "Add filter", "Columns" and a filter chip. The chip's × stays 24px.
- **A wide table never widens the screen.** It scrolls sideways inside itself; the stage's one column is `minmax(0, 1fr)`. (Before this, the count and the Columns button sat past the window's right edge.)
- **The default columns fit without sideways scrolling at a 1270px window with the sidebar open** (a test holds it). The widths are minimums; Title and Artist then take the room that is left (3 : 2, Genre 1, File and Folder 1 each), and the numbers stay narrow: Title 132, Artist 90, BPM 54, Key 46, Time 50, Genre 60, Rating 76, Date added 90, File 60, Folder 60. With each screen's actions column that is 830px for the Library, 938px for All music and 718px for a crate, whose pane is narrower (760px: the crate list beside it is 14rem).
- **The sorted column's heading reads at full strength**, the others muted; the caret says the direction.
- **A row's title is read before its notes:** the notes (file missing, in Downloads) shrink first, the title only when nothing is left of them.
- **The Columns panel has no filled button.** A change applies the moment it is made, so "Done" only closes the panel and is quiet. The filled accent button is for the action a screen leads to (Keep the suggested file, Send), not for dismissing a panel. If the panel ever needs a confirm (Apply), that one would be filled.
- **One selected item in the sidebar.** With a crate chosen in the tree, the crate carries the tint and the bar; the Crates stage above it keeps its place without them. The tree indents 16px a level.
- **Empty states are centred** where the table would be, like the other empty states.

## The one exception to "class names only"

A cell that is cut off with "…" gets a `title` attribute (the native hover text) set to **the same value the cell shows**: no new text, no new data, no locale key. Allowed by the foreman, 2026-10-10. A test per list holds it ("a cut-off title cell carries its full text").

Hover text does not reach keyboard users, and a touch screen has no hover. So the full text must also live in the Details panel when that panel is built (today it is a placeholder; showing a track's title and path there is a new feature, logged by the foreman for the Phase 1a checkpoint). Until then the full title is only on hover.

## How the decisions look in the styles

- **Text:** `--font-size-md` 14px; `--font-size-sm` 12px. A test holds those floors.
- **Stripes and selected:** three steps, each a clear step stronger than the last: a stripe (faintest), hover, selected (strongest). A test holds the order in both themes, and that text stays readable on each.
- **Selected** is the warm tint `--color-selected` on the sidebar's active stage and the chosen crate, with the accent bar or border kept.
- **Cut-off:** Library and All music keep a row to one line (`table-layout: fixed`, title 28% and artist 18% of the width, a fixed actions column); a long title, artist or path ends in "…". Rows are at the compact row height; a row with a note under its path is taller on purpose.
- **Crate tree (1cA-11):** the sidebar lists folders and crates under the Crates stage, each level indented by `--space-3`, a small arrow button (open/close) before each folder and the same row height, tint and accent bar as a stage for the chosen one. A folder that holds the chosen crate stays open. On the Crates screen the same tree has folder names in bold and track counts on crates only. The rekordbox path sits under the chosen name in muted small text.
- **Motion:** one hover fade (`--motion-fast`), colors only, set on hover so a theme switch stays instant. The only animation is the playing bars (`PlayingBars.module.css`): one keyframes rule that changes `transform` alone, switched off under "reduce motion". Tests check both.

## Text that has to be read

`--color-text-disabled` is only for controls that can't be used. Text the user needs to read (an empty state such as "Nothing playing" or "No crates") is `--color-text-muted`. A test holds it.

## Audio preview (decided 2026-10-16)

- 2026-10-16 · **Play and Pause are the standard icons** (a triangle, and two bars), not words. Each button keeps its name as its accessible label and hover text (`title`, the same string, no new wording). · Quicker to scan in a long list. · Style (drawing), tests read the name instead of the visible word
- 2026-10-16 · **A playing track is shown by three small bars that alternate in height** (an equalizer), in the row of the file that is playing and in the player bar. Built as `PlayingBars` (decorative, hidden from screen readers, no text, no data; it takes its room even when hidden, so nothing moves when it appears), with the foreman's yes of 2026-10-16. It is the one animation in the app: three bars rising and falling out of step in the accent colour (playing is "active"), by `transform` alone, and still (at different heights) when Windows' "reduce motion" is on. The playing row is **not** tinted (see the next decision).

- 2026-10-16 · **Selected and playing are two different things, and only selected is tinted.** A track can be selected (to see its metadata, to act on it) while another one plays, so the warm tint (`--color-selected`) and the accent bar mean "selected" and nothing else. Playing is shown only by the bars. · Tinting the playing row would be confused with the selected one. · Style (a rule for every list, the Details panel and the queue)

## Open questions for the owner

None yet.
