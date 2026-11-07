# Releasing

Steps for making a release. The script is `scripts/release.mjs`; it needs Node, git, the [GitHub CLI](https://cli.github.com) (`gh`, signed in) and the build tools from the README. It never publishes: you do that on GitHub.

1. **Bump the version.** On a branch, run `node scripts/release.mjs bump 0.2.0` (the new number must be higher than the current one). It edits the five places that hold the version and lists them. Open a pull request, merge it.
2. **Update your checkout.** Switch to `main`, pull, and make sure `git status` shows nothing changed.
3. **Build.** `node scripts/release.mjs build --out C:\releases`. It refuses if the tree is dirty, if you are not at the latest `origin/main`, or if the file already exists. It then builds the installer (the same command as in [install-check.md](install-check.md)), copies it to `tracklist-pro_<version>_x64-setup.exe` in that folder, writes a `.sha256` file next to it, and prints the size and sha256. Build from a folder outside OneDrive.
4. **Draft.** `node scripts/release.mjs draft --out C:\releases`. It creates a draft release `v<version>` on GitHub with the installer and the `.sha256` attached, and notes that link [installing.md](installing.md) and say the installer is unsigned (SmartScreen: More info, Run anyway). It refuses if the tag or release already exists.
5. **Publish.** Open the draft on GitHub, read the notes, and press Publish release yourself. Nothing else publishes it.

If a step fails, it says why and changes nothing further. To redo a build, delete the files it names first.
