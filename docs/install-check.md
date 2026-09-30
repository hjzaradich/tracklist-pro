# Side-by-side install check

Checks that an installed tracklist-pro and musicmanager stay separate on one Windows PC (ROADMAP §1.1 "musicmanager", §5.6 "Coexisting with musicmanager"). Re-run it when the bundle ID, installer settings, updater or credential storage change, and before a release.

Rules while checking: only look at musicmanager's files (list names, sizes and times). Never write to, move or delete anything of musicmanager's, and don't build or change musicmanager.

## 1. Build the installer
From the repo (not a OneDrive folder):

```
npm ci
npx tauri build --bundles nsis
```

The installer lands in `src-tauri\target\release\bundle\nsis\tracklist-pro_<version>_x64-setup.exe`.

If the build fails with `LNK1207: incompatible PDB format` (left behind by a crash or power loss), delete `src-tauri\target\release` and build again.

## 2. Note how musicmanager runs
Either an installed build (it has a Start-menu entry and an Installed-apps entry) or a dev build run from its project folder. A dev build shows "localhost refused to connect" without its dev server. That's fine here: the program still starts and opens its data folders. With a dev build, the installer checks in step 5 are "not applicable" for musicmanager.

## 3. Snapshot before
Close both apps. Record a recursive listing (relative path, size, modified time) of each folder that exists:

- `%APPDATA%\com.musicmanager.desktop`
- `%LOCALAPPDATA%\com.musicmanager.desktop`
- `%APPDATA%\com.tracklistpro.desktop`
- `%LOCALAPPDATA%\com.tracklistpro.desktop`

For example, in PowerShell, per folder:

```
Get-ChildItem $root -Recurse -Force | ForEach-Object { [pscustomobject]@{ Path=$_.FullName.Substring($root.Length); Size=$(if($_.PSIsContainer){''}else{[string]$_.Length}); Modified=$_.LastWriteTimeUtc.ToString('o') } } | Export-Csv before.csv -NoTypeInformation
```

## 4. Install and run both
1. Run the installer with the defaults. A locally built file shows no SmartScreen warning; a downloaded one shows "Windows protected your PC" (More info → Run anyway).
2. Leave tracklist-pro open and start musicmanager. Both must run at the same time (both processes appear in Task Manager).
3. Close both apps.

## 5. Check separation
- **Data:** tracklist-pro wrote only to its two `com.tracklistpro.desktop` folders; its program files are in `%LOCALAPPDATA%\tracklist-pro`.
- **musicmanager's data:** compare with the snapshot. `library.db` is unchanged. Changes allowed are only those musicmanager makes itself while running, all timestamped after it started: `library.db-shm` and its WebView2 profile (`EBWebView\…`).
- **Start menu:** tracklist-pro has its own `tracklist-pro` shortcut; musicmanager's (if installed) is separate.
- **Installed apps:** separate `tracklist-pro` and musicmanager entries (if installed).
- **Updater and credentials:** grep `src`, `src-tauri/src`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` for `updater`, `keyring` and `credential`. Once either exists, confirm its endpoint or prefix differs from musicmanager's.

## 6. Uninstall and re-check
1. Take a fresh snapshot of musicmanager's folders (both apps closed).
2. Settings → Apps → Installed apps → tracklist-pro → Uninstall. Leave "Delete the application data" unticked.
3. Check: the Installed-apps entry, the shortcuts and `%LOCALAPPDATA%\tracklist-pro` are gone. The `com.tracklistpro.desktop` data folders stay (expected), and so does the installer's own `HKCU\Software\tracklistpro` key (install path and language only).
4. musicmanager's folders match the fresh snapshot exactly.
5. Start musicmanager again; it still starts.
