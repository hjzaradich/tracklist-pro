# Installing tracklist-pro

How to build the Windows installer, install it, and remove it again. Windows 10 and 11 only (ROADMAP §1.1). There is no download yet: the installer is built from this repo.

Checked on 2026-10-01 by building version 0.1.0. The build steps and file names below are from that build. The install and uninstall steps weren't run then; they follow the installer the build generates and [install-check.md](install-check.md), which was run on an earlier build.

## 1. Build the installer

You need Node.js, Rust (stable, MSVC toolchain) and the Microsoft C++ Build Tools. Keep the repo outside OneDrive.

From the repo's top folder:

```
npm ci
npm run tauri build
```

The first build takes a while (about 30 minutes on a laptop). It downloads the two installer tools it needs (NSIS and WiX) the first time.

If it fails with `LNK1207: incompatible PDB format` (left behind by a crash or power loss), delete `src-tauri\target\release` and build again.

## 2. Where the installer lands

The build makes two installers. Use the first. `<version>` is the app's version, 0.1.1 at the time of writing.

| File | What it is |
|---|---|
| `src-tauri\target\release\bundle\nsis\tracklist-pro_<version>_x64-setup.exe` | **The one to use** (about 5 MB). Installs for your Windows account only and needs no administrator rights. |
| `src-tauri\target\release\bundle\msi\tracklist-pro_<version>_x64_en-US.msi` | An MSI (about 7 MB), for the whole PC. It asks for administrator rights. Not needed. |

To build only the first one: `npx tauri build --bundles nsis`.

Neither file is committed to the repo.

## 3. Install

Run `tracklist-pro_<version>_x64-setup.exe` and keep the defaults. The program goes to `%LOCALAPPDATA%\tracklist-pro`, with a `tracklist-pro` entry in the Start menu.

### The "Windows protected your PC" warning

**Not checked on this build yet.** The wording below is what Windows usually shows for an unsigned installer; it wasn't seen on this build, and Windows versions differ.

tracklist-pro isn't code-signed (signing costs money, and the project is free; ROADMAP §1.1). Windows SmartScreen warns about any unsigned installer that came from the internet:

- A blue window titled **Windows protected your PC**, saying Microsoft Defender SmartScreen prevented an unrecognized app from starting. It shows a single **Don't run** button.
- Click **More info** (a link in the text). The window then shows the file's name and "Publisher: Unknown publisher", and a second button.
- Click **Run anyway**.

An installer you built yourself on the same PC doesn't show the warning. A copy that was downloaded, or sent to you, does. Only run one you built or got from someone you trust.

Before SmartScreen, a browser may also hold back the download ("isn't commonly downloaded"). Choose to keep the file.

### Installing over an older version

**Not run on this build.** The steps below are read from the installer the build generates.

When an older tracklist-pro is already installed, the installer says so and offers two choices:

- **Do not uninstall**: the new version is installed over the old one. Your Library and everything else in the app's data folders (section 4) are kept.
- **Uninstall before installing**: the old version's uninstaller runs first, then the new version is installed. That uninstaller shows a box, **Delete the application data**. Leave it unticked. Ticked, it deletes both folders in section 4, your Library with them, and that can't be undone.

Either choice works. **Do not uninstall** never shows that box.

The first time the new version starts, it brings your existing database up to date by itself. Nothing needs doing, and your music files aren't touched.

## 4. Where your data is kept

| Folder | What's in it |
|---|---|
| `%APPDATA%\com.tracklistpro.desktop\` | Everything the app keeps: its database (`tracklist-pro.db`) and the file it writes for rekordbox (`tracklist-pro.xml`). |
| `%LOCALAPPDATA%\com.tracklistpro.desktop\` | The embedded browser's own cache (WebView2). Nothing of yours. |

To open the first one, paste `%APPDATA%\com.tracklistpro.desktop` into File Explorer's address bar.

The app writes nowhere else. Your music files, your music folders and rekordbox's own files are only read.

The folder is named after the app's bundle ID, `com.tracklistpro.desktop` (ROADMAP §1.1). The installer carries the same ID, and a test checks that the app resolves its data folder to `%APPDATA%\com.tracklistpro.desktop`.

## 5. Uninstall

1. Close tracklist-pro.
2. Settings → Apps → Installed apps → tracklist-pro → Uninstall.
3. The uninstaller offers **Delete the application data**. Leave it unticked to keep your Library for a later install. Tick it to delete both folders in section 4; that can't be undone.

Uninstalling removes the program folder and the Start-menu entry. It never touches your music files or rekordbox.

To remove the data later by hand, delete the two folders in section 4.
