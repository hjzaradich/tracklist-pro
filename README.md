# tracklist-pro

A companion for DJs who use rekordbox on Windows. It scans your music folders, helps you clean up duplicates, and sends a tidy Library to rekordbox as an XML file.

## Status

Early. Not ready for use.

## What it's for

- Scans your music folders. It only reads them.
- Finds duplicate files and related versions (remixes, edits, clean and dirty).
- Keeps a Library that stays in step with rekordbox through its XML import and export.
- Never moves, renames, retags or deletes your music.

How to use it: the [user guide](docs/guide.md). How to install it, including the Windows "protected your PC" warning: [installing](docs/installing.md). The plan is in [ROADMAP.md](ROADMAP.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

## Not affiliated

rekordbox, AlphaTheta, Pioneer DJ and the other product names here are trademarks of their owners. tracklist-pro isn't affiliated with or endorsed by any of them. See [NOTICE.md](NOTICE.md).

## Privacy

- No telemetry: the app never sends usage data or analytics.
- Every online service is off until you turn it on, one by one.
- Your music stays on your computer. The app never uploads audio.
- Every request goes through one gate that only talks to the services below, at the addresses listed, and only when you've turned that service on.
- What each service receives:
  - **MusicBrainz** (musicbrainz.org): the title, artist or ID it's looking up.
  - **Discogs** (api.discogs.com): the search you ask for, with your own token.
  - **Cover Art Archive** (coverartarchive.org, and archive.org where it hands images over): the release it's fetching art for.
  - **Update checks** and **model downloads**: the gate has an on/off switch for each, but no address is set yet, so every request to them is refused.
- Every request also carries the app's name, its version and the project's repository address (in the User-Agent), and your IP address reaches the service, as with any website.
- More services will be listed here before they are added.

Right now no feature contacts any service.

## Contributions

Issues and ideas welcome; I'm not accepting code contributions right now.

## Build

You need Windows 10 or 11, [Node.js](https://nodejs.org), [Rust](https://rustup.rs) and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) (Visual Studio C++ build tools and WebView2).

```
npm ci
npm run tauri dev
```

Tests: `npm test` for the interface, `cargo test` in `src-tauri` for the rest.
