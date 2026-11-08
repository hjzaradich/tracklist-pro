# Notice

## License

tracklist-pro is free software under the GNU General Public License, version 3 or any later version (GPL-3.0-or-later). See [LICENSE](LICENSE). It comes with no warranty.

## Not affiliated

rekordbox, AlphaTheta, Pioneer DJ, CDJ, XDJ, OneLibrary, Spotify, Beatport and any other names here are trademarks of their owners. They're used only to say what tracklist-pro works with. It isn't affiliated with or endorsed by any of them.

## Privacy

No telemetry. Every online service is off until you turn it on, and each says what it sends. Your music never leaves your computer. The details are in the [README](README.md#privacy).

## Credits

- **libKeyFinder** (https://github.com/mixxxdj/libkeyfinder), Copyright 2011-2015 Ibrahim Sha'ath, and the Mixxx contributors, GPL-3.0-or-later. The app's own key estimate (`src-tauri/src/estimate/key/`) is a translation of its method into Rust: the low-pass filter (based on Tony Fisher's mkfilter work), the chroma spectrum, and the tone profiles from Sha'ath's 2011 thesis. Each translated file names the files it comes from.
- **CED-tiny** (https://huggingface.co/mispeech/ced-tiny), Copyright 2023 Xiaomi Corporation and The HuggingFace Inc. team, Apache-2.0. The app's optional audio model, shipped in the installer as `src-tauri/resources/models/ced-tiny/` (converted to ONNX with an extra embedding output; its licence and notice are in that folder). Paper: Dinkel et al., "CED: Consistent ensemble distillation for audio tagging", ICASSP 2024. Its label names are the AudioSet ontology's (Google, CC BY-SA 4.0).
