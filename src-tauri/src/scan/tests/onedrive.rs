//! The OneDrive check: where a folder is, never a change to it.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::scan::onedrive::{has_cloud_attributes, is_onedrive_folder, under_onedrive_root};
use crate::scan::online_only::ATTRIBUTE_RECALL_ON_DATA_ACCESS;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(*value))
    }
}

#[test]
fn a_path_under_a_onedrive_environment_folder_is_detected() {
    for variable in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        let pairs = [(variable, r"C:\Users\dj\OneDrive")];
        let lookup = env(&pairs);
        assert!(under_onedrive_root(r"C:\Users\dj\OneDrive\Music", &lookup));
        assert!(under_onedrive_root(r"c:\users\DJ\onedrive", &lookup));
        assert!(under_onedrive_root(
            r"\\?\C:\Users\dj\OneDrive\Music\",
            &lookup
        ));
    }
}

#[test]
fn a_plain_folder_is_not_flagged() {
    let lookup = env(&[("OneDrive", r"C:\Users\dj\OneDrive")]);
    assert!(!under_onedrive_root(r"D:\DJ Music", &lookup));
    // A sibling that merely starts with the same letters.
    assert!(!under_onedrive_root(
        r"C:\Users\dj\OneDrive Old\Music",
        &lookup
    ));
    // No variable set, or an empty one: nothing to match.
    assert!(!under_onedrive_root(
        r"C:\Users\dj\OneDrive\Music",
        env(&[])
    ));
    assert!(!under_onedrive_root(r"C:\Music", env(&[("OneDrive", "")])));

    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.mp3"), b"x").unwrap();
    assert!(!is_onedrive_folder(dir.path(), env(&[])));
}

#[test]
fn cloud_attributes_on_a_file_in_the_folder_are_detected() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("local.mp3"), b"x").unwrap();
    fs::write(dir.path().join("cloud.mp3"), b"x").unwrap();
    let read = |path: &Path| {
        let cloud = path.file_name().is_some_and(|name| name == "cloud.mp3");
        Some(if cloud {
            ATTRIBUTE_RECALL_ON_DATA_ACCESS
        } else {
            0
        })
    };
    assert!(has_cloud_attributes(dir.path(), read));
    assert!(!has_cloud_attributes(dir.path(), |_: &Path| Some(0)));
    // The folder's own attributes count too.
    let own = dir.path().to_path_buf();
    assert!(has_cloud_attributes(dir.path(), move |path: &Path| {
        Some(if path == own {
            ATTRIBUTE_RECALL_ON_DATA_ACCESS
        } else {
            0
        })
    }));
}

#[test]
fn only_a_small_sample_of_the_files_is_asked_about() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..60 {
        fs::write(dir.path().join(format!("{n:02}.mp3")), b"x").unwrap();
    }
    let asked = std::cell::Cell::new(0);
    let none = has_cloud_attributes(dir.path(), |_: &Path| {
        asked.set(asked.get() + 1);
        Some(0)
    });
    assert!(!none);
    assert!(asked.get() <= 25, "asked about {} paths", asked.get());
}
