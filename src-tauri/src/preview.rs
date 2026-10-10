//! The development preview: a debug build that opens a made-up sample
//! library instead of the real app data folder, so the owner can watch the
//! screens change while the designer edits them.
//!
//! **Debug builds only.** `lib.rs` declares this module under
//! `#[cfg(debug_assertions)]`, so a release build has none of it: it never
//! reads [`ENV_VAR`] and always opens the real folder. A test reads the
//! sources and holds that line.
//!
//! The preview lives in one fixed folder, [`ROOT`], marked by [`MARKER`]
//! (`npm run design` makes it and fills it with generated sample data).
//! [`ENV_VAR`] only asks for it. It is never a way to point the app at some
//! other folder: anything but that marked folder's `data` folder is refused,
//! and a refusal stops the app. It never falls back to the real folder
//! without saying so.
//!
//! Inside the folder:
//!
//! - `data\`: the app data folder (database, send file).
//! - `music\`: the generated music folder the sample library scans.
//! - `documents\`: where the preview looks for a rekordbox export, instead
//!   of the real Documents folder.
//! - `webview\`: the window's own browser profile, so the preview shares no
//!   saved state with the installed app.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// The environment variable that asks for the preview.
pub const ENV_VAR: &str = "TLP_PREVIEW_DIR";

/// The preview's folder. `scripts/preview.mjs` holds the same path (a test
/// checks they agree); it is the only folder the script ever deletes.
pub const ROOT: &str = r"C:\dev\tracklist-pro-preview";

/// The file that says a folder is the preview's: nothing is written into
/// or deleted from a folder without it.
pub const MARKER: &str = "tlp-preview-folder.txt";

/// Written last by the seed. Without it the sample data is half made and
/// `npm run design` starts again.
pub const SEEDED: &str = "seeded.txt";

/// The preview window's title. Not user copy: it only tells the preview
/// from the installed app.
pub const WINDOW_TITLE: &str = "tracklist-pro (preview)";

/// A checked preview folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    root: PathBuf,
}

/// Why a folder isn't accepted as the preview's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The variable is set but empty.
    Empty,
    /// A relative path (the app's working folder isn't a fixed place).
    NotAbsolute,
    /// The preview folder isn't there, or isn't the folder it should be
    /// (a link to somewhere else).
    NoFolder(PathBuf),
    /// The folder has no [`MARKER`]: it isn't the preview's.
    NoMarker(PathBuf),
    /// The real app data folder.
    RealAppData,
    /// Anything else: only the preview's `data` folder is accepted.
    NotTheDataFolder,
    /// The `data` folder isn't there yet: the sample data was never made.
    NotSeeded(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Empty => write!(f, "{ENV_VAR} is set but empty"),
            Refusal::NotAbsolute => write!(f, "{ENV_VAR} must be an absolute path"),
            Refusal::NoFolder(root) => {
                write!(f, "the preview folder {} isn't there", root.display())
            }
            Refusal::NoMarker(root) => write!(
                f,
                "{} has no {MARKER}, so it isn't the preview's folder",
                root.display()
            ),
            Refusal::RealAppData => write!(f, "{ENV_VAR} names the real app data folder"),
            Refusal::NotTheDataFolder => write!(
                f,
                "{ENV_VAR} must be the data folder inside the preview folder"
            ),
            Refusal::NotSeeded(data) => write!(
                f,
                "{} isn't there yet: the sample data hasn't been made",
                data.display()
            ),
        }
    }
}

/// The path as Windows spells it without the `\\?\` that `canonicalize`
/// adds.
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Whether two paths are the same spelling, case apart (Windows paths).
fn same(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    key(a) == key(b)
}

impl Preview {
    /// The preview folder at `root`: it exists, is that folder and not a
    /// link to another, and holds the [`MARKER`]. The app passes [`ROOT`];
    /// tests pass a temp folder.
    pub fn at(root: &Path) -> Result<Preview, Refusal> {
        if !root.is_absolute() {
            return Err(Refusal::NotAbsolute);
        }
        let real = fs::canonicalize(root)
            .map(plain)
            .map_err(|_| Refusal::NoFolder(root.to_owned()))?;
        // A junction or link would put the preview somewhere else.
        if !real.is_dir() || !same(&real, root) {
            return Err(Refusal::NoFolder(root.to_owned()));
        }
        let marker = fs::symlink_metadata(real.join(MARKER));
        if !marker.is_ok_and(|m| m.is_file()) {
            return Err(Refusal::NoMarker(real));
        }
        Ok(Preview { root: real })
    }

    /// The preview `value` (the variable's value) asks for, if it's the
    /// marked folder's `data` folder and nothing else. `real_app_data` is
    /// the real app data folder, named so a mix-up gets its own message.
    pub fn check(
        root: &Path,
        value: &OsStr,
        real_app_data: Option<&Path>,
    ) -> Result<Preview, Refusal> {
        if value.is_empty() {
            return Err(Refusal::Empty);
        }
        let asked = Path::new(value);
        if !asked.is_absolute() {
            return Err(Refusal::NotAbsolute);
        }
        if let Some(real) = real_app_data {
            let resolved = fs::canonicalize(asked).map(plain).ok();
            if same(asked, real) || resolved.is_some_and(|r| same(&r, real)) {
                return Err(Refusal::RealAppData);
            }
        }
        let preview = Preview::at(root)?;
        let data = preview.data();
        if !data.is_dir() {
            return if same(asked, &data) {
                Err(Refusal::NotSeeded(data))
            } else {
                Err(Refusal::NotTheDataFolder)
            };
        }
        let resolved = fs::canonicalize(asked)
            .map(plain)
            .map_err(|_| Refusal::NotTheDataFolder)?;
        if !same(&resolved, &data) {
            return Err(Refusal::NotTheDataFolder);
        }
        Ok(preview)
    }

    /// The preview [`ENV_VAR`] asks for: `None` when it isn't set, and an
    /// error when it asks for anything but the marked [`ROOT`]'s data
    /// folder.
    pub fn from_env() -> Result<Option<Preview>, Refusal> {
        Preview::from_var(std::env::var_os(ENV_VAR).as_deref())
    }

    /// [`Preview::from_env`] for a given value of the variable.
    fn from_var(value: Option<&OsStr>) -> Result<Option<Preview>, Refusal> {
        let Some(value) = value else {
            return Ok(None);
        };
        let real = dirs::data_dir().map(|d| d.join(crate::IDENTIFIER));
        Preview::check(Path::new(ROOT), value, real.as_deref()).map(Some)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The app data folder.
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    /// The generated music folder.
    pub fn music(&self) -> PathBuf {
        self.root.join("music")
    }

    /// Where the preview looks for rekordbox's export.
    pub fn documents(&self) -> PathBuf {
        self.root.join("documents")
    }

    /// The window's own browser profile.
    pub fn webview(&self) -> PathBuf {
        self.root.join("webview")
    }

    /// Whether the seed ran to its end.
    pub fn is_seeded(&self) -> bool {
        self.root.join(SEEDED).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A marked preview folder in a temp folder, as the app would find it.
    fn marked() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = plain(fs::canonicalize(dir.path()).unwrap()).join("preview");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join(MARKER), "made by npm run design\n").unwrap();
        (dir, root)
    }

    fn check(root: &Path, value: impl AsRef<OsStr>) -> Result<Preview, Refusal> {
        Preview::check(root, value.as_ref(), None)
    }

    #[test]
    fn the_marked_data_folder_is_accepted() {
        let (_dir, root) = marked();
        let preview = check(&root, root.join("data")).unwrap();
        assert_eq!(preview.data(), root.join("data"));
        assert_eq!(preview.documents(), root.join("documents"));
        assert_eq!(preview.music(), root.join("music"));
    }

    #[test]
    fn spelling_differences_that_name_the_same_folder_are_accepted() {
        let (_dir, root) = marked();
        let data = root.join("data");
        assert!(check(&root, root.join("data").join("..").join("data")).is_ok());
        // Windows spells folders with a backslash and folds case; others
        // don't.
        if cfg!(windows) {
            let slash = format!("{}\\", data.display());
            assert!(check(&root, &slash).is_ok(), "{slash}");
            let upper = data.to_string_lossy().to_uppercase();
            assert!(check(&root, &upper).is_ok(), "{upper}");
        }
    }

    #[test]
    fn an_empty_value_is_refused() {
        let (_dir, root) = marked();
        assert_eq!(check(&root, ""), Err(Refusal::Empty));
    }

    #[test]
    fn a_relative_path_is_refused() {
        let (_dir, root) = marked();
        for relative in ["data", r"..\preview\data", "./data", ""] {
            assert!(check(&root, relative).is_err(), "{relative:?}");
        }
        assert_eq!(check(&root, "data"), Err(Refusal::NotAbsolute));
    }

    #[test]
    fn any_folder_but_the_marked_data_folder_is_refused() {
        let (dir, root) = marked();
        let other = dir.path().join("elsewhere");
        fs::create_dir_all(&other).unwrap();
        for bad in [
            other.clone(),
            root.clone(),
            root.join("music"),
            root.join("data").join("sub"),
            root.join("data").join("..").join("music"),
            dir.path().to_owned(),
        ] {
            let got = check(&root, &bad);
            assert!(got.is_err(), "{} was accepted", bad.display());
        }
        assert_eq!(check(&root, &other), Err(Refusal::NotTheDataFolder));
    }

    #[test]
    fn the_real_app_data_folder_is_refused_by_name() {
        let (dir, root) = marked();
        let real = dir.path().join("Roaming").join(crate::IDENTIFIER);
        fs::create_dir_all(&real).unwrap();
        assert_eq!(
            Preview::check(&root, real.as_os_str(), Some(&real)),
            Err(Refusal::RealAppData)
        );
        // A spelling that resolves to it is the same folder.
        assert_eq!(
            Preview::check(
                &root,
                real.join("..").join(crate::IDENTIFIER).as_os_str(),
                Some(&real)
            ),
            Err(Refusal::RealAppData)
        );
    }

    #[test]
    fn a_folder_without_the_marker_is_refused_whatever_it_holds() {
        let (_dir, root) = marked();
        fs::remove_file(root.join(MARKER)).unwrap();
        assert!(matches!(
            check(&root, root.join("data")),
            Err(Refusal::NoMarker(_))
        ));
        // A folder in place of the marker file doesn't count.
        fs::create_dir(root.join(MARKER)).unwrap();
        assert!(matches!(
            check(&root, root.join("data")),
            Err(Refusal::NoMarker(_))
        ));
    }

    #[test]
    fn a_missing_preview_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = plain(fs::canonicalize(dir.path()).unwrap()).join("nothing-here");
        assert!(matches!(
            check(&root, root.join("data")),
            Err(Refusal::NoFolder(_))
        ));
    }

    #[test]
    fn a_marked_folder_without_sample_data_says_so() {
        let (_dir, root) = marked();
        fs::remove_dir(root.join("data")).unwrap();
        assert!(matches!(
            check(&root, root.join("data")),
            Err(Refusal::NotSeeded(_))
        ));
    }

    #[test]
    fn the_seed_is_complete_only_when_its_last_file_is_there() {
        let (_dir, root) = marked();
        let preview = Preview::at(&root).unwrap();
        assert!(!preview.is_seeded());
        fs::write(root.join(SEEDED), "").unwrap();
        assert!(preview.is_seeded());
    }

    #[test]
    fn an_unset_variable_means_the_real_folder_and_a_set_one_is_checked_against_root() {
        assert_eq!(Preview::from_var(None), Ok(None));
        // Whatever the variable says, it's checked against ROOT: a folder
        // that isn't the marked preview is refused, never accepted.
        let elsewhere = std::env::temp_dir().join("tlp-not-the-preview");
        assert!(Preview::from_var(Some(elsewhere.as_os_str())).is_err());
        assert!(Preview::from_var(Some(OsStr::new(""))).is_err());
        assert!(ROOT.ends_with("tracklist-pro-preview"));
    }

    /// Every `.rs` file under `dir`.
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn only_a_debug_build_can_read_the_variable() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&src, &mut files);
        let name = ["TLP_PREVIEW", "_DIR"].concat();
        let naming: Vec<_> = files
            .iter()
            .filter(|f| fs::read_to_string(f).unwrap().contains(&name))
            .map(|f| f.strip_prefix(&src).unwrap().to_owned())
            .collect();
        // The module that reads it, and nowhere else (this test builds the
        // name in two pieces, so it doesn't count).
        assert_eq!(naming, [PathBuf::from("preview.rs")]);

        // That module is compiled in debug builds only, and the startup
        // reaches it only from code compiled the same way.
        let lib = fs::read_to_string(src.join("lib.rs"))
            .unwrap()
            .replace("\r\n", "\n");
        assert!(
            lib.contains("#[cfg(debug_assertions)]\npub mod preview;"),
            "lib.rs must declare `mod preview` under cfg(debug_assertions)"
        );
        for gate in ["debug_assertions", "not(debug_assertions)"] {
            assert!(
                lib.contains(&format!(
                    "#[cfg({gate})]
fn startup_data_dir"
                )),
                "startup_data_dir needs a form under cfg({gate})"
            );
        }
    }

    #[test]
    fn the_preview_window_says_so_and_keeps_its_own_browser_profile() {
        // The mock runtime can't report a window's title, so the source
        // is read: the preview's window is built with them, and only for
        // a preview.
        let lib = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .unwrap()
            .replace("\r\n", "\n");
        let rest = &lib[lib.find("fn open_windows").unwrap()..];
        let body = &rest[..rest.find("\n}\n").unwrap()];
        assert!(body.contains("DataDir::Preview(preview)"), "{body}");
        assert!(body.contains(".title(preview::WINDOW_TITLE)"), "{body}");
        assert!(body.contains(".data_directory("), "{body}");
        assert!(WINDOW_TITLE.contains("preview"));
    }

    #[test]
    fn the_script_deletes_the_same_folder_the_app_opens() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("preview.mjs");
        let text = fs::read_to_string(script).unwrap();
        let literal = ROOT.replace('\\', "\\\\");
        assert!(
            text.contains(&literal),
            "scripts/preview.mjs must hold {ROOT}"
        );
        assert!(
            text.contains(MARKER),
            "scripts/preview.mjs must hold {MARKER}"
        );
        assert!(
            text.contains(SEEDED),
            "scripts/preview.mjs must hold {SEEDED}"
        );
    }
}
