//! The generator writes into the folder it's given and nowhere else.

mod common;

use std::fs;

use common::{fresh_dir, walk};
use fixture_gen::sandbox::{RelPath, SandboxError, Target};
use fixture_gen::{generate, Error, Options};

fn core_only() -> Options {
    Options::default()
}

#[test]
fn writes_nothing_outside_the_target_folder() {
    let parent = fresh_dir("outside-check");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("keep.txt"), b"untouched").unwrap();
    let target = parent.join("fixtures");

    let summary = generate(&target, &core_only()).unwrap();

    // Beside the target: exactly what was there before.
    let mut beside: Vec<_> = fs::read_dir(&parent)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    beside.sort();
    assert_eq!(beside, ["fixtures", "keep.txt"]);
    assert_eq!(fs::read(parent.join("keep.txt")).unwrap(), b"untouched");
    // Inside: the files it reported, plus the manifest.
    let inside = walk(&fs::canonicalize(&target).unwrap());
    assert_eq!(inside.len(), summary.files + 1);
}

#[test]
fn refuses_a_target_folder_that_already_has_files() {
    let dir = fresh_dir("not-empty");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("mine.mp3"), b"the user's file").unwrap();

    let err = generate(&dir, &core_only()).unwrap_err();

    assert!(
        matches!(err, Error::Sandbox(SandboxError::NotEmpty(_))),
        "{err}"
    );
    let names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["mine.mp3"], "something was added");
    assert_eq!(fs::read(dir.join("mine.mp3")).unwrap(), b"the user's file");
}

#[test]
fn refuses_a_target_whose_parent_folder_is_missing_and_creates_neither() {
    let base = fresh_dir("no-parent");
    fs::create_dir(&base).unwrap();
    let target = base.join("missing").join("fixtures");

    let err = generate(&target, &core_only()).unwrap_err();

    assert!(
        matches!(err, Error::Sandbox(SandboxError::NoParent(_))),
        "{err}"
    );
    assert!(!base.join("missing").exists());
}

#[test]
fn refuses_a_target_that_is_a_file() {
    let base = fresh_dir("file-target");
    fs::create_dir(&base).unwrap();
    let file = base.join("song.mp3");
    fs::write(&file, b"x").unwrap();

    let err = generate(&file, &core_only()).unwrap_err();

    assert!(
        matches!(err, Error::Sandbox(SandboxError::NotAFolder(_))),
        "{err}"
    );
    assert_eq!(fs::read(&file).unwrap(), b"x");
}

#[test]
fn relative_paths_that_could_leave_the_target_are_refused() {
    for bad in [
        "../escape.mp3",
        "music/../../escape.mp3",
        "C:/escape.mp3",
        "C:escape.mp3",
        "music\\..\\..\\escape.mp3",
        "",
        "music//x.mp3",
    ] {
        let err = RelPath::parse(bad).unwrap_err();
        assert!(matches!(err, SandboxError::BadPath(_)), "{bad:?}: {err}");
    }
}

#[test]
fn an_existing_file_is_never_overwritten() {
    let dir = fresh_dir("no-overwrite");
    let target = Target::create(&dir).unwrap();
    let rel = RelPath::parse("a.mp3").unwrap();
    target.write_new(&rel, b"first").unwrap();

    assert!(target.write_new(&rel, b"second").is_err());
    assert_eq!(fs::read(target.resolve(&rel)).unwrap(), b"first");
}
