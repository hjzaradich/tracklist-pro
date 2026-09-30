//! 0E-8, part 2: run the app's startup and every read path it has today
//! against a sandbox, then check that nothing else in the sandbox (outside
//! its app data folder) changed, and nothing in the working directory
//! either: no file's bytes or modified time, no folder's modified time, no
//! entry added or removed (ROADMAP §5.6).
//!
//! The system temp folder isn't watched: other tests run alongside this one
//! and create temp dirs there all the time. SQLite's only way into it, its
//! temp files, is closed by `temp_store = MEMORY`, which the db and guard
//! tests check on every connection.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tauri::Manager;

use super::WriteGuard;
use crate::tags::test_audio::Format;
use crate::{db, tags, volume, DataDir};

/// What's recorded about each entry.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File {
        bytes: Vec<u8>,
        modified: SystemTime,
        readonly: bool,
    },
    Folder {
        modified: SystemTime,
    },
    /// A symlink or junction; not followed.
    Link {
        modified: SystemTime,
    },
}

/// Everything under `dir` except the `skip` paths, keyed by path.
fn snapshot(dir: &Path, skip: &[&Path], out: &mut BTreeMap<PathBuf, Entry>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if skip.contains(&path.as_path()) {
            continue;
        }
        let meta = fs::symlink_metadata(&path).unwrap();
        let modified = meta.modified().unwrap();
        let kind = meta.file_type();
        // Junctions aren't symlinks to `FileType`, but `is_dir` on a link's
        // own metadata is false only for real symlinks; check both.
        let is_link = kind.is_symlink() || fs::read_link(&path).is_ok();
        let entry = if is_link {
            Entry::Link { modified }
        } else if kind.is_dir() {
            snapshot(&path, skip, out);
            Entry::Folder { modified }
        } else {
            Entry::File {
                bytes: fs::read(&path).unwrap(),
                modified,
                readonly: meta.permissions().readonly(),
            }
        };
        out.insert(path, entry);
    }
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        out.push(path.clone());
        if path.is_dir() && fs::read_link(&path).is_err() {
            files_under(&path, out);
        }
    }
}

/// A music folder like a DJ's: every audio format the tag reader handles,
/// a non-audio file, a read-only file, a name Win32 would rewrite, and a
/// linked folder. Every file's modified time is put in the past, so any
/// write would move it.
fn music_folder(base: &Path) -> PathBuf {
    let music = base.join("music");
    let artist = music.join("Artist");
    fs::create_dir_all(&artist).unwrap();
    for format in Format::ALL {
        format.write_to(&artist, "Track");
    }
    fs::write(artist.join("notes.txt"), b"not audio").unwrap();
    let locked = Format::Mp3.write_to(&music, "Locked");
    if cfg!(windows) {
        // `base` is in `\\?\` form, so the trailing dot is kept.
        let rem = music.join("Q.X.Z.");
        fs::create_dir(&rem).unwrap();
        Format::Flac.write_to(&rem, "Song One");
    }
    link_dir(&music.join("Linked"), &artist);

    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let mut files = Vec::new();
    files_under(&music, &mut files);
    for file in files.iter().filter(|f| f.is_file()) {
        fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let mut perms = fs::metadata(&locked).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&locked, perms).unwrap();
    music
}

fn link_dir(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink: {made:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// Every read path the app has today, on one file or folder.
fn read_everything_about(path: &Path) {
    let _ = tags::is_audio_file(path);
    if path.is_file() {
        let _ = tags::read(path);
    }
    let _ = volume::volume_for(path);
    #[cfg(windows)]
    {
        use crate::paths::{StoredPath, SystemVolumes};
        let volumes = SystemVolumes::scan();
        if let Ok(stored) = StoredPath::from_absolute(path, &volumes) {
            let resolved = stored.resolve(&volumes).unwrap();
            if resolved.is_file() {
                fs::read(&resolved).unwrap();
            }
        }
    }
}

#[test]
#[allow(deprecated)]
fn startup_and_every_read_path_leave_the_rest_of_the_sandbox_and_the_working_dir_unchanged() {
    let sandbox = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(sandbox.path()).unwrap();
    let data = base.join("data");
    let music = music_folder(&base);

    // The working directory too (the crate folder under `cargo test`),
    // leaving out the build output that other test binaries may touch.
    let cwd = std::env::current_dir().unwrap();
    let build_output = cwd.join("target");
    let watch = |out: &mut BTreeMap<PathBuf, Entry>| {
        snapshot(&base, &[&data], out);
        snapshot(&cwd, &[&build_output], out);
    };

    let mut before = BTreeMap::new();
    watch(&mut before);
    assert!(
        before.keys().filter(|p| p.starts_with(&base)).count() > 10,
        "the sandbox is too small: {before:?}"
    );
    assert!(
        before.keys().any(|p| p.starts_with(&cwd)),
        "nothing found in the working directory {cwd:?}"
    );

    {
        // Startup: creates the data folder, opens and migrates the database.
        let mut app = crate::tests::mock_app_with(DataDir::At(data.clone()));
        app.run_iteration(|_, _| {});

        // The database, read and written.
        let reader = app.state::<db::ReadPool>();
        reader.read(db::migrations::applied).unwrap();
        let writer = app.state::<db::Writer>();
        writer
            .call(|c| c.execute_batch("CREATE TEMP TABLE t (x); INSERT INTO t VALUES (1);"))
            .unwrap();
        // A write the app is allowed to make.
        let guard = app.state::<WriteGuard>();
        guard.write(&data.join("state.json"), b"{}").unwrap();

        // Every file and folder in the music folder, through every reader.
        read_everything_about(&music);
        let mut paths = Vec::new();
        files_under(&music, &mut paths);
        assert!(paths.len() >= Format::ALL.len() + 3, "{paths:?}");
        for path in &paths {
            read_everything_about(path);
        }
        // The app (and its database connections) close here.
    }

    let mut after = BTreeMap::new();
    watch(&mut after);
    let changed: Vec<String> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .map(|p| {
            let short = |e: Option<&Entry>| match e {
                None => "missing".to_owned(),
                Some(Entry::File {
                    bytes, modified, ..
                }) => format!("{} bytes, modified {modified:?}", bytes.len()),
                Some(other) => format!("{other:?}"),
            };
            format!(
                "{}: {} → {}",
                p.display(),
                short(before.get(p)),
                short(after.get(p))
            )
        })
        .collect();
    assert!(
        changed.is_empty(),
        "outside the data folder, these changed:\n{}",
        changed.join("\n")
    );

    // The writes that did happen landed inside the data folder.
    assert!(data.join(db::DB_FILE_NAME).is_file());
    assert_eq!(fs::read(data.join("state.json")).unwrap(), b"{}");

    // Let the temp dir clean up.
    let locked = music.join("Locked.mp3");
    let mut perms = fs::metadata(&locked).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&locked, perms).unwrap();
}
