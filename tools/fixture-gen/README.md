# fixture-gen (dev-only)

Writes a synthetic music folder for tests and performance runs: small, valid
audio files with made-up tags, plus `fixture-manifest.json`, the ground truth
later tests assert against. Not part of the app; nothing in `src-tauri`
depends on it.

```
cargo run --release -- --out <empty folder> [--seed N] [--files N] [--bulk-ms N] [--threads N]
```

- `--out` must be new or empty, and its parent must exist. Nothing is written
  anywhere else, and no existing file is ever overwritten.
- The same `--seed` always writes the same bytes (on the same platform and
  toolchain).
- `--files` is the total file count. The ground-truth cases (about 70 files)
  are always written; bulk files fill up to the count. `0` gives the cases
  alone.

## What it writes

`<out>/fixture-manifest.json` and `<out>/music/`. Point scanners at `music/`.

- **Duplicates:** one recording as FLAC, WAV, AIFF, M4A and MP3 at several
  bitrates, with different tags, rip-junk names (site-name prefixes and
  suffixes), a store-style ID-prefixed name with HTML-escaped parentheses, a
  copy that differs only in its ID3 tag, a rip with extra lead-in silence, and
  untagged copies.
- **Versions:** Extended and Radio Edit cuts (exact slices of the original,
  recorded in `version_links[].contains`), Clean/Dirty cuts, and remix, VIP,
  bootleg, flip, cover, live and mashup reworks. Includes E3's hard cases: a
  remixer-only credit, a same-length bootleg, a version word that's part of
  the title (`Velo (dirty) (dirty)`), and a live take tagged exactly like the
  studio original.
- **Unrelated look-alikes** (`not_related`): same title with different songs,
  and a mashup whose part name matches an unrelated track.
- **Awkward names:** a trailing-dot folder (`Q.V.X.`) and a trailing-space
  folder (`Drift Unit `), each beside a look-alike sibling holding a different
  file; `CON.mp3` and `AUX.mp3`; NFC/NFD twins; emoji (with a ZWJ sequence);
  Japanese and Korean; `# % + & '`; a literal `%20`; uppercase extensions; a
  path over 260 characters.
- **Skip cases:** `._` AppleDouble files and `.DS_Store`.
- **Traps:** a `.wav` holding MP3 bytes, an M4A with no `moov` box, an ID3v2.4
  `TDRC` of `2019-13-45`, an APEv2 footer claiming 16 MB and 7 items, and an
  empty file.
- **A gig-stick copy:** byte-identical copies under
  `Old USB Backup/Contents/<Artist>/<Album>/`.
- **Quality cases** (ROADMAP 1.6): MP3 320 and FLAC whose content stops at
  16 kHz (expected `suspect_transcode`), and full-band ones (expected `ok`).
- **Bulk files:** 0.5 s, 22.05 kHz tones, each different, spread over
  `Library/<Genre>/<Artist>/<Album>/` and one big flat `Downloads/Bulk/`.
  About 62% MP3, 12% FLAC, 9% WAV, 8% AIFF, 9% M4A.

All core audio is 44.1 kHz mono, 16-bit.

## Formats

| Format | How | Notes |
|---|---|---|
| MP3 | LAME 3.100 via `mp3lame-encoder` (LGPL) | CBR, with the LAME/Info frame so decoders trim delay and padding exactly |
| FLAC | Hand-written | Constant, fixed order-2 (Rice-coded) or verbatim subframes; MD5 left as "not computed" |
| WAV | Hand-written | 16-bit PCM, `LIST`/`INFO` tags |
| AIFF | Hand-written | 16-bit PCM, `ID3 ` chunk |
| M4A | Hand-written | **Apple Lossless, not AAC.** No GPL-compatible AAC encoder is small enough to add (FDK AAC's license is GPL-incompatible). ALAC frames use the format's uncompressed escape. |

## Size and time

On a typical Windows 11 laptop (release build, NTFS, Defender on):

| Files | Size | Time |
|---|---|---|
| ~70 (cases only) | 16 MB | 4 s |
| 1,000 | 28 MB | 8 s |
| 100,000 | 1.34 GB (1.6 GB on disk) | 407 s |

Large runs are disk-bound, not CPU-bound: creating and scanning 100k new files
costs more than encoding them.

## Deleting a tree

The trailing-dot and trailing-space folders can't be deleted through a plain
Windows path (Explorer and `Remove-Item` fail). Use a `\\?\` path:

```
cmd /c rd /s /q "\\?\C:\full\path\to\out"
```

## Tests

`cargo test` generates small trees and checks them with an independent decoder
(Symphonia): every file marked valid decodes to exactly its recorded length,
every trap is broken in the intended way, names are exact on disk, duplicate
audio matches, versions don't, the same seed gives byte-identical output, and
nothing is written outside the target. CI runs them on Windows.
