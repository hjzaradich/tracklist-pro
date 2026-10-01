//! What the guard allows and refuses. Every refused write is also checked
//! to have left the disk untouched.

use super::*;
use std::path::Path;

/// A sandbox with the guard's root at `<sandbox>/data` and a sibling
/// folder `<sandbox>/outside` holding one file, `keep.txt`.
struct Sandbox {
    _dir: tempfile::TempDir,
    base: PathBuf,
    guard: WriteGuard,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        // Resolved, so paths built from it read the same as the guard's.
        let base = fs::canonicalize(dir.path()).unwrap();
        let guard = WriteGuard::app_data(&base.join("data")).unwrap();
        fs::create_dir(base.join("outside")).unwrap();
        fs::write(base.join("outside").join("keep.txt"), b"keep").unwrap();
        Sandbox {
            _dir: dir,
            base,
            guard,
        }
    }

    fn data(&self) -> PathBuf {
        self.base.join("data")
    }

    fn outside(&self) -> PathBuf {
        self.base.join("outside")
    }

    /// Every name under `outside`, to show a refused write created nothing.
    fn outside_listing(&self) -> Vec<String> {
        let mut names = Vec::new();
        list(&self.outside(), "", &mut names);
        names.sort();
        names
    }

    /// Tries every kind of write at `path` and asserts each is refused and
    /// that nothing under `outside` changed.
    fn assert_every_write_refused(&self, path: &Path) {
        let before = self.outside_listing();
        let keep = self.outside().join("keep.txt");
        let refused = |what: &str, result: Result<(), GuardError>| {
            assert!(result.is_err(), "{what} at {path:?} was allowed");
        };
        refused("check", self.guard.check(path).map(drop));
        refused("create_dir_all", self.guard.create_dir_all(path).map(drop));
        refused("create_file", self.guard.create_file(path).map(drop));
        refused(
            "open_for_writing",
            self.guard.open_for_writing(path).map(drop),
        );
        refused("write", self.guard.write(path, b"x"));
        refused(
            "write_then_rename",
            self.guard.write_then_rename(path, b"x"),
        );
        refused("remove_file", self.guard.remove_file(path));
        refused("remove_dir_all", self.guard.remove_dir_all(path));
        let inside = self.data().join("inside.txt");
        self.guard.write(&inside, b"in").unwrap();
        refused("rename in", self.guard.rename(&inside, path));
        refused("rename out", self.guard.rename(path, &inside));
        assert_eq!(self.outside_listing(), before, "{path:?}");
        assert_eq!(fs::read(&keep).unwrap(), b"keep");
    }
}

fn list(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        let kind = entry.file_type().unwrap();
        out.push(name.clone());
        if kind.is_dir() && !kind.is_symlink() {
            list(&entry.path(), &format!("{name}/"), out);
        }
    }
}

/// A directory junction on Windows (no admin rights needed), a symlink
/// elsewhere.
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

// ---- inside the root -------------------------------------------------------

#[test]
fn every_kind_of_write_inside_the_root_succeeds() {
    let sb = Sandbox::new();
    let guard = &sb.guard;
    let nested = sb.data().join("a").join("b");
    guard.create_dir_all(&nested).unwrap();
    assert!(nested.is_dir());

    let file = nested.join("one.txt");
    guard.write(&file, b"one").unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"one");

    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = guard.open_for_writing(&file).unwrap();
        f.seek(SeekFrom::End(0)).unwrap();
        f.write_all(b"+more").unwrap();
    }
    assert_eq!(fs::read(&file).unwrap(), b"one+more");

    {
        use std::io::Write;
        guard.create_file(&file).unwrap().write_all(b"new").unwrap();
    }
    assert_eq!(fs::read(&file).unwrap(), b"new");

    let moved = sb.data().join("moved.txt");
    guard.rename(&file, &moved).unwrap();
    assert!(!file.exists());
    assert_eq!(fs::read(&moved).unwrap(), b"new");

    guard.remove_file(&moved).unwrap();
    assert!(!moved.exists());

    guard.remove_dir_all(&sb.data().join("a")).unwrap();
    assert!(!sb.data().join("a").exists());
    assert!(sb.data().is_dir());
}

#[test]
fn the_guard_creates_its_missing_data_folder_and_parents_and_can_be_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("nested").join(crate::IDENTIFIER);
    assert!(!data.exists());
    let guard = WriteGuard::app_data(&data).unwrap();
    assert!(data.is_dir());
    assert_eq!(guard.app_data_dir(), fs::canonicalize(&data).unwrap());
    assert_eq!(guard.root(RootKind::AppData), Some(guard.app_data_dir()));
    // Again, on the folder that now exists.
    WriteGuard::app_data(&data).unwrap();
}

#[test]
fn a_checked_path_is_the_resolved_one_inside_the_root() {
    let sb = Sandbox::new();
    let checked = sb.guard.check(&sb.data().join("x").join("y.db")).unwrap();
    assert_eq!(checked.root(), RootKind::AppData);
    assert!(
        within(checked.as_path(), sb.guard.app_data_dir()),
        "{checked:?}"
    );
    assert!(checked.as_path().ends_with(Path::new("x").join("y.db")));
    // The root itself is inside the root.
    sb.guard.check(&sb.data()).unwrap();
}

#[test]
fn a_dot_dot_that_stays_inside_the_root_is_allowed_and_resolved() {
    let sb = Sandbox::new();
    sb.guard.create_dir_all(&sb.data().join("a")).unwrap();
    let detour = sb.data().join("a").join("..").join("b.txt");
    sb.guard.write(&detour, b"b").unwrap();
    assert_eq!(fs::read(sb.data().join("b.txt")).unwrap(), b"b");
    assert_eq!(
        sb.guard.check(&detour).unwrap(),
        sb.guard.check(&sb.data().join("b.txt")).unwrap()
    );
}

#[test]
fn opening_a_database_creates_it_inside_the_root_with_temp_data_in_memory() {
    let sb = Sandbox::new();
    let conn = sb
        .guard
        .check(&sb.data().join("app.db"))
        .unwrap()
        .open_database()
        .unwrap();
    conn.execute_batch("CREATE TABLE t (x INTEGER)").unwrap();
    let temp_store: i64 = conn
        .query_row("PRAGMA temp_store", [], |r| r.get(0))
        .unwrap();
    assert_eq!(temp_store, 2, "temp_store is not MEMORY");
    assert!(sb.data().join("app.db").is_file());
}

// ---- outside the root ------------------------------------------------------

#[test]
fn a_path_outside_the_root_is_refused() {
    let sb = Sandbox::new();
    sb.assert_every_write_refused(&sb.outside().join("keep.txt"));
    sb.assert_every_write_refused(&sb.outside().join("new.txt"));
    sb.assert_every_write_refused(&sb.outside());
    sb.assert_every_write_refused(&sb.base);
}

#[test]
fn a_sibling_whose_name_merely_starts_like_the_root_is_refused() {
    let sb = Sandbox::new();
    let evil = sb.base.join("data-evil");
    fs::create_dir(&evil).unwrap();
    assert!(sb.guard.check(&evil.join("x.db")).is_err());
    assert!(sb.guard.write(&evil.join("x.db"), b"x").is_err());
    assert!(!evil.join("x.db").exists());
}

#[test]
fn dot_dot_climbing_out_of_the_root_is_refused() {
    let sb = Sandbox::new();
    let climb = sb.data().join("..").join("outside").join("new.txt");
    sb.assert_every_write_refused(&climb);
    // Through a folder inside the root that doesn't exist yet.
    let deeper = sb
        .data()
        .join("nope")
        .join("..")
        .join("..")
        .join("outside")
        .join("keep.txt");
    sb.assert_every_write_refused(&deeper);
}

#[test]
fn a_linked_folder_inside_the_root_that_points_outside_is_refused() {
    let sb = Sandbox::new();
    let link = sb.data().join("link");
    link_dir(&link, &sb.outside());
    sb.assert_every_write_refused(&link.join("keep.txt"));
    sb.assert_every_write_refused(&link.join("new.txt"));
    sb.assert_every_write_refused(&link.join("new").join("deeper.txt"));
    // Writing through the link itself is refused too.
    assert!(sb.guard.check(&link).is_err());
    assert!(sb.guard.create_dir_all(&link).is_err());
}

#[test]
fn renaming_or_deleting_a_link_in_the_root_acts_on_the_link_not_what_it_points_to() {
    let sb = Sandbox::new();
    let before = sb.outside_listing();
    let link = sb.data().join("link");
    link_dir(&link, &sb.outside());
    let moved = sb.data().join("moved");
    sb.guard.rename(&link, &moved).unwrap();
    assert!(fs::symlink_metadata(&moved).is_ok() && !link.exists());
    sb.guard.remove_dir_all(&moved).unwrap();
    assert!(
        fs::symlink_metadata(&moved).is_err(),
        "the link is still there"
    );
    assert_eq!(sb.outside_listing(), before);
    assert_eq!(fs::read(sb.outside().join("keep.txt")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn deleting_a_file_link_in_the_root_leaves_its_target() {
    let sb = Sandbox::new();
    let target = sb.data().join("target.txt");
    sb.guard.write(&target, b"target").unwrap();
    let link = sb.data().join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    sb.guard.remove_file(&link).unwrap();
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"target");
    // A link pointing outside can be deleted too, leaving its target.
    let out = sb.data().join("out.txt");
    std::os::unix::fs::symlink(sb.outside().join("keep.txt"), &out).unwrap();
    sb.guard.remove_file(&out).unwrap();
    assert_eq!(fs::read(sb.outside().join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn a_link_inside_the_root_that_points_nowhere_is_refused() {
    // Writing through a link whose target doesn't exist would create the
    // target, wherever it is.
    let sb = Sandbox::new();
    let gone = sb.outside().join("gone");
    let link = sb.data().join("dangling");
    fs::create_dir(&gone).unwrap();
    link_dir(&link, &gone);
    fs::remove_dir(&gone).unwrap();
    match sb.guard.check(&link.join("x.txt")) {
        Err(GuardError::Unresolvable { .. }) => {}
        other => panic!("expected Unresolvable, got {other:?}"),
    }
    sb.assert_every_write_refused(&link.join("x.txt"));
    assert!(!gone.exists());
}

#[test]
fn deleting_a_folder_in_the_root_removes_a_link_in_it_but_not_what_it_points_to() {
    let sb = Sandbox::new();
    let folder = sb.data().join("folder");
    sb.guard.create_dir_all(&folder).unwrap();
    link_dir(&folder.join("link"), &sb.outside());
    sb.guard.remove_dir_all(&folder).unwrap();
    assert!(!folder.exists());
    assert_eq!(fs::read(sb.outside().join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn a_root_cannot_be_deleted_or_moved() {
    let sb = Sandbox::new();
    for result in [
        sb.guard.remove_dir_all(&sb.data()),
        sb.guard.rename(&sb.data(), &sb.data().join("elsewhere")),
    ] {
        assert!(
            matches!(result, Err(GuardError::IsRoot(_))),
            "got {result:?}"
        );
    }
    assert!(sb.data().is_dir());
}

#[test]
fn a_relative_path_is_refused() {
    let sb = Sandbox::new();
    for rel in ["x.db", "data/x.db", "../x.db", ""] {
        match sb.guard.check(Path::new(rel)) {
            Err(GuardError::NotAbsolute(_)) => {}
            other => panic!("{rel:?}: expected NotAbsolute, got {other:?}"),
        }
    }
}

#[test]
fn a_whole_drive_cannot_be_a_root() {
    let root = if cfg!(windows) {
        let dir = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(dir.path()).unwrap();
        // `\\?\C:\…` → `C:\`
        PathBuf::from(format!("{}\\", &real.to_str().unwrap()[4..6]))
    } else {
        PathBuf::from("/")
    };
    match WriteGuard::app_data(&root) {
        Err(GuardError::RootTooBroad(_)) => {}
        other => panic!("{root:?}: expected RootTooBroad, got {other:?}"),
    }
}

// ---- Windows path forms ----------------------------------------------------

#[cfg(windows)]
mod windows {
    use super::*;

    /// `\\?\C:\…` → `C:\…`.
    fn plain(path: &Path) -> PathBuf {
        PathBuf::from(path.to_str().unwrap().strip_prefix(r"\\?\").unwrap())
    }

    #[test]
    fn the_base_is_in_verbatim_form() {
        let sb = Sandbox::new();
        assert!(sb.base.to_str().unwrap().starts_with(r"\\?\"));
    }

    #[test]
    fn plain_verbatim_device_and_forward_slash_forms_inside_the_root_are_allowed() {
        let sb = Sandbox::new();
        let verbatim = sb.data().join("a.txt");
        let expected = sb.guard.check(&verbatim).unwrap();
        let text = plain(&verbatim).to_str().unwrap().to_owned();
        for form in [
            text.clone(),
            format!(r"\\.\{text}"),
            text.replace('\\', "/"),
            text.to_uppercase(),
        ] {
            let checked = sb.guard.check(Path::new(&form)).unwrap();
            assert!(
                crate::paths::is_within(checked.as_path(), expected.as_path())
                    && crate::paths::is_within(expected.as_path(), checked.as_path()),
                "{form}: {checked:?} vs {expected:?}"
            );
        }
        sb.guard.write(&plain(&verbatim), b"plain").unwrap();
        assert_eq!(fs::read(&verbatim).unwrap(), b"plain");
    }

    #[test]
    fn verbatim_and_device_forms_outside_the_root_are_refused() {
        let sb = Sandbox::new();
        let outside = plain(&sb.outside().join("keep.txt"));
        let text = outside.to_str().unwrap();
        sb.assert_every_write_refused(Path::new(&format!(r"\\?\{text}")));
        sb.assert_every_write_refused(Path::new(&format!(r"\\.\{text}")));
        // `..` inside a `\\?\` path is resolved before the check, even
        // though Win32 wouldn't resolve it there.
        let climb = format!(r"{}\..\outside\new.txt", sb.data().to_str().unwrap());
        assert!(climb.starts_with(r"\\?\"));
        sb.assert_every_write_refused(Path::new(&climb));
    }

    #[test]
    fn device_namespace_and_volume_guid_paths_are_refused() {
        let sb = Sandbox::new();
        for path in [
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\x.txt",
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x.txt",
            r"\\.\pipe\x",
        ] {
            assert!(
                sb.guard.check(Path::new(path)).is_err(),
                "{path} was allowed"
            );
        }
    }

    #[test]
    fn another_drive_is_refused() {
        let sb = Sandbox::new();
        let own = sb.base.to_str().unwrap().as_bytes()[4];
        // Every other letter: whether or not a drive is mounted there,
        // nothing on it is inside the root.
        for letter in (b'A'..=b'Z').filter(|l| *l != own.to_ascii_uppercase()) {
            let letter = letter as char;
            for path in [format!(r"{letter}:\x.txt"), format!(r"\\?\{letter}:\x.txt")] {
                assert!(
                    sb.guard.check(Path::new(&path)).is_err(),
                    "{path} was allowed"
                );
            }
        }
    }

    #[test]
    fn the_same_path_on_another_drive_letter_is_refused() {
        // `D:\…\data\x.txt` mirrors the root's own path on another drive.
        let sb = Sandbox::new();
        let own = sb.data().join("x.txt");
        let text = own.to_str().unwrap();
        let other = if text.as_bytes()[4].eq_ignore_ascii_case(&b'Z') {
            'Y'
        } else {
            'Z'
        };
        let mirrored = format!(r"{other}{}", &text[5..]);
        assert!(sb.guard.check(Path::new(&mirrored)).is_err(), "{mirrored}");
    }

    #[test]
    fn a_folder_whose_name_ends_in_a_dot_is_not_confused_with_its_dotless_sibling() {
        // Win32 turns `data.\x` into `data\x` in a plain path. A guard on
        // `data.` must not let a write reach `data`, and vice versa.
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let dotted = WriteGuard::app_data(&base.join("data.")).unwrap();
        fs::create_dir(base.join("data")).unwrap();
        assert!(dotted.check(&base.join("data").join("x.txt")).is_err());
        dotted
            .write(&base.join("data.").join("x.txt"), b"x")
            .unwrap();
        assert!(base.join("data.").join("x.txt").is_file());
        assert!(!base.join("data").join("x.txt").exists());
    }

    #[test]
    fn a_unc_path_to_the_root_through_an_admin_share_is_refused() {
        // `\\localhost\C$\…` reaches the same folder by another route; the
        // guard only accepts the drive-letter form it resolved to.
        let sb = Sandbox::new();
        let text = sb.data().join("x.txt").to_str().unwrap().to_owned();
        let unc = format!(r"\\localhost\{}${}", &text[4..5], &text[6..]);
        assert!(sb.guard.check(Path::new(&unc)).is_err(), "{unc}");
    }
}

// ---- write, then rename ----------------------------------------------------

/// Every name in the data folder.
fn data_listing(sb: &Sandbox) -> Vec<String> {
    let mut names = Vec::new();
    list(&sb.data(), "", &mut names);
    names.sort();
    names
}

#[test]
fn write_then_rename_leaves_only_the_destination() {
    let sb = Sandbox::new();
    let dest = sb.data().join("send.xml");
    sb.guard.write_then_rename(&dest, b"whole").unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"whole");
    assert_eq!(data_listing(&sb), ["send.xml"]);
}

#[test]
fn write_then_rename_replaces_an_existing_destination_with_the_complete_file() {
    let sb = Sandbox::new();
    let dest = sb.data().join("send.xml");
    sb.guard.write(&dest, b"old and longer").unwrap();
    // While the new file is being filled, the destination still holds
    // its old content.
    sb.guard
        .write_then_rename_with(
            &dest,
            |file| {
                use std::io::Write;
                file.write_all(b"ne")?;
                assert_eq!(fs::read(&dest).unwrap(), b"old and longer");
                file.write_all(b"w")
            },
            || {
                assert_eq!(fs::read(&dest).unwrap(), b"old and longer");
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"new");
    assert_eq!(data_listing(&sb), ["send.xml"]);
}

#[test]
fn a_write_that_fails_midway_leaves_no_destination_and_no_temp_file() {
    let sb = Sandbox::new();
    let dest = sb.data().join("send.xml");
    let failed = sb.guard.write_then_rename_with(
        &dest,
        |file| {
            use std::io::Write;
            file.write_all(b"half")?;
            Err(io::Error::other("disk full"))
        },
        || Ok(()),
    );
    assert!(matches!(failed, Err(GuardError::Io { .. })), "{failed:?}");
    assert_eq!(data_listing(&sb), Vec::<String>::new());
}

#[test]
fn a_failure_before_the_rename_keeps_the_old_destination_and_leaves_no_temp_file() {
    let sb = Sandbox::new();
    let dest = sb.data().join("send.xml");
    sb.guard.write(&dest, b"old").unwrap();
    let failed = sb.guard.write_then_rename_with(
        &dest,
        |file| {
            use std::io::Write;
            file.write_all(b"new")
        },
        || Err(io::Error::other("stopped")),
    );
    assert!(matches!(failed, Err(GuardError::Io { .. })), "{failed:?}");
    assert_eq!(fs::read(&dest).unwrap(), b"old");
    assert_eq!(data_listing(&sb), ["send.xml"]);
}

#[test]
fn a_folder_in_the_way_of_the_destination_is_refused_and_nothing_is_created() {
    // The destination names a folder, which is never replaced.
    let sb = Sandbox::new();
    let dest = sb.data().join("send.xml");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("inside.txt"), b"kept").unwrap();
    assert!(sb.guard.write_then_rename(&dest, b"new").is_err());
    assert_eq!(data_listing(&sb), ["send.xml", "send.xml/inside.txt"]);
    assert_eq!(fs::read(dest.join("inside.txt")).unwrap(), b"kept");
}

#[test]
fn write_then_rename_never_opens_or_deletes_a_file_that_already_has_its_temp_name() {
    let sb = Sandbox::new();
    let dest = sb.data().join("send.xml");
    let taken = sb.data().join("send.xml.part");
    fs::write(&taken, b"someone's").unwrap();

    sb.guard.write_then_rename(&dest, b"new").unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"new");
    assert_eq!(fs::read(&taken).unwrap(), b"someone's");

    // A failed write deletes its own temp file, not that one.
    let failed =
        sb.guard
            .write_then_rename_with(&dest, |_| Err(io::Error::other("disk full")), || Ok(()));
    assert!(failed.is_err());
    assert_eq!(fs::read(&taken).unwrap(), b"someone's");
    assert_eq!(fs::read(&dest).unwrap(), b"new");
    assert_eq!(data_listing(&sb), ["send.xml", "send.xml.part"]);
}

#[test]
fn write_then_rename_refuses_a_destination_outside_the_root_before_creating_anything() {
    let sb = Sandbox::new();
    let before = sb.outside_listing();
    let dest = sb.outside().join("send.xml");
    let filled = std::cell::Cell::new(false);
    let refused = sb.guard.write_then_rename_with(
        &dest,
        |_| {
            filled.set(true);
            Ok(())
        },
        || Ok(()),
    );
    assert!(
        matches!(refused, Err(GuardError::Outside(_))),
        "{refused:?}"
    );
    assert!(
        !filled.get(),
        "nothing is created for a refused destination"
    );
    assert_eq!(sb.outside_listing(), before);
    assert_eq!(data_listing(&sb), Vec::<String>::new());
}

#[test]
fn write_then_rename_refuses_a_hard_linked_destination_before_creating_anything() {
    let sb = Sandbox::new();
    let outside = sb.outside().join("keep.txt");
    let linked = sb.data().join("linked.txt");
    fs::hard_link(&outside, &linked).unwrap();
    assert!(matches!(
        sb.guard.write_then_rename(&linked, b"PWNED"),
        Err(GuardError::HardLinked(_))
    ));
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
    assert_eq!(data_listing(&sb), ["linked.txt"]);
}

// ---- hard links ------------------------------------------------------------

#[test]
fn a_hard_link_in_the_root_to_an_outside_file_is_never_written_through() {
    // Review finding: `write` through `data/linked.txt` changed the file's
    // other name outside the root.
    let sb = Sandbox::new();
    let outside = sb.outside().join("keep.txt");
    let linked = sb.data().join("linked.txt");
    fs::hard_link(&outside, &linked).unwrap();
    let before = fs::metadata(&outside).unwrap().modified().unwrap();

    let refused = |what: &str, result: Result<(), GuardError>| match result {
        Err(GuardError::HardLinked(_)) => {}
        other => panic!("{what}: expected HardLinked, got {other:?}"),
    };
    refused("write", sb.guard.write(&linked, b"PWNED"));
    refused("create_file", sb.guard.create_file(&linked).map(drop));
    refused(
        "open_for_writing",
        sb.guard.open_for_writing(&linked).map(drop),
    );
    // Nothing was written or emptied, and the time didn't move.
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
    assert_eq!(fs::metadata(&outside).unwrap().modified().unwrap(), before);

    // Deleting the extra name only removes that name.
    sb.guard.remove_file(&linked).unwrap();
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
    // With one name again, the file inside the root is writable as usual.
    let own = sb.data().join("own.txt");
    sb.guard.write(&own, b"one").unwrap();
    sb.guard.write(&own, b"two").unwrap();
    assert_eq!(fs::read(&own).unwrap(), b"two");
}

#[test]
fn a_hard_link_between_two_names_inside_the_root_is_refused_too() {
    // The guard can't tell where a file's other names are, so it refuses
    // any file with more than one.
    let sb = Sandbox::new();
    let a = sb.data().join("a.txt");
    sb.guard.write(&a, b"a").unwrap();
    fs::hard_link(&a, sb.data().join("b.txt")).unwrap();
    assert!(matches!(
        sb.guard.write(&a, b"x"),
        Err(GuardError::HardLinked(_))
    ));
    assert_eq!(fs::read(&a).unwrap(), b"a");
}

#[test]
fn a_database_whose_files_are_hard_linked_is_not_opened() {
    let sb = Sandbox::new();
    // An existing database outside, and a second name for it inside.
    let outside_db = sb.outside().join("foreign.db");
    drop(
        rusqlite::Connection::open(&outside_db)
            .unwrap()
            .execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);"),
    );
    let bytes = fs::read(&outside_db).unwrap();
    let inside_db = sb.data().join("app.db");
    fs::hard_link(&outside_db, &inside_db).unwrap();
    let checked = sb.guard.check(&inside_db).unwrap();
    assert!(checked.open_database().is_err(), "read-write open");
    assert!(checked.open_database_read_only().is_err(), "read-only open");
    assert_eq!(fs::read(&outside_db).unwrap(), bytes);

    // A hard-linked side file (`-wal`) is refused the same way.
    fs::remove_file(&inside_db).unwrap();
    let own = sb.data().join("own.db");
    let wal_outside = sb.outside().join("stray-wal");
    fs::write(&wal_outside, b"").unwrap();
    fs::hard_link(&wal_outside, sb.data().join("own.db-wal")).unwrap();
    assert!(sb.guard.check(&own).unwrap().open_database().is_err());
}

// ---- SQLite: no ATTACH, no VACUUM INTO -------------------------------------

/// Every way the review found to make SQLite open another file.
fn attach_attempts(target: &Path) -> Vec<(String, Option<String>)> {
    let t = target.to_str().unwrap().replace('\'', "''");
    let quoted = format!("'{t}'");
    vec![
        (format!("ATTACH {quoted} AS x"), None),
        (format!("ATTACH DATABASE {quoted} AS x"), None),
        (format!("ATTACH\n{quoted} AS x"), None),
        (format!("attach lower({quoted}) as x"), None),
        ("ATTACH ?1 AS x".to_owned(), Some(t.clone())),
        ("ATTACH @x AS x".to_owned(), Some(t.clone())),
        (format!("ATTACH 'file:{t}?mode=rwc' AS x"), None),
        (format!("VACUUM INTO {quoted}"), None),
        (format!("VACUUM main INTO {quoted}"), None),
    ]
}

/// Runs each attempt on `conn` and asserts it's refused and no file (or
/// side file) appeared.
fn assert_no_attach(conn: &rusqlite::Connection, target: &Path) {
    for (sql, param) in attach_attempts(target) {
        let result = match param {
            None => conn.execute_batch(&sql),
            Some(p) if sql.contains('@') => conn
                .execute(&sql, rusqlite::named_params! {"@x": p})
                .map(drop),
            Some(p) => conn.execute(&sql, [p]).map(drop),
        };
        assert!(result.is_err(), "{sql:?} was allowed");
        let parent = target.parent().unwrap();
        let name = target.file_name().unwrap().to_string_lossy().into_owned();
        let appeared: Vec<_> = fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&name))
            .collect();
        assert!(appeared.is_empty(), "{sql:?} created {appeared:?}");
    }
}

#[test]
fn the_guards_database_connections_refuse_attach_and_vacuum_into() {
    let sb = Sandbox::new();
    let checked = sb.guard.check(&sb.data().join("app.db")).unwrap();
    let conn = checked.open_database().unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (x); INSERT INTO t VALUES (1);")
        .unwrap();
    assert_no_attach(&conn, &sb.outside().join("attached.db"));
    // Plain VACUUM still works: its scratch database stays in memory.
    conn.execute_batch("VACUUM").unwrap();

    let reader = checked.open_database_read_only().unwrap();
    assert_no_attach(&reader, &sb.outside().join("attached.db"));
    // Even a read-only attach of an existing WAL database would write
    // `-wal`/`-shm` beside it (review finding).
    let foreign = sb.outside().join("foreign.db");
    drop(
        rusqlite::Connection::open(&foreign)
            .unwrap()
            .execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE f (x);"),
    );
    let before = sb.outside_listing();
    let sql = format!(
        "ATTACH 'file:{}?mode=ro' AS f",
        foreign.to_str().unwrap().replace('\'', "''")
    );
    assert!(reader.execute_batch(&sql).is_err());
    assert_eq!(sb.outside_listing(), before);
}

#[test]
fn sql_cannot_move_sqlites_temp_or_data_files() {
    // Review round 3: `PRAGMA temp_store = FILE` (or a temp directory)
    // would send SQLite's temp files back to %TEMP% or anywhere else.
    let sb = Sandbox::new();
    let checked = sb.guard.check(&sb.data().join("app.db")).unwrap();
    let writer = checked.open_database().unwrap();
    writer.execute_batch("CREATE TABLE t (x)").unwrap();
    let reader = checked.open_database_read_only().unwrap();
    let elsewhere = sb.outside().to_str().unwrap().replace('\'', "''");
    for conn in [&writer, &reader] {
        for sql in [
            "PRAGMA temp_store = FILE".to_owned(),
            "PRAGMA temp_store = 1".to_owned(),
            "pragma TEMP_STORE(DEFAULT)".to_owned(),
            "PRAGMA main.temp_store = 0".to_owned(),
            format!("PRAGMA temp_store_directory = '{elsewhere}'"),
            format!("PRAGMA data_store_directory = '{elsewhere}'"),
        ] {
            let _ = conn.execute_batch(&sql);
            let temp_store: i64 = conn
                .query_row("PRAGMA temp_store", [], |r| r.get(0))
                .unwrap();
            assert_eq!(temp_store, 2, "{sql} changed temp_store");
        }
        // Setting it the way the guard already has it is refused too; only
        // reading is allowed.
        assert!(conn.execute_batch("PRAGMA temp_store = MEMORY").is_err());
        let dir: Option<String> = conn
            .query_row("PRAGMA temp_store_directory", [], |r| r.get(0))
            .unwrap_or(None);
        assert!(dir.unwrap_or_default().is_empty(), "temp directory was set");
    }
}

#[test]
fn a_database_side_file_that_is_a_link_is_not_opened() {
    // Review round 3: `File::open` follows links, so the link-count check
    // alone would pass a `-wal` that's a symlink to a file outside.
    let sb = Sandbox::new();
    // A folder (or a junction, which is a folder link) in a side file's
    // place is refused.
    let db = sb.data().join("dir.db");
    fs::create_dir(sb.data().join("dir.db-wal")).unwrap();
    let checked = sb.guard.check(&db).unwrap();
    assert!(checked.open_database().is_err());
    assert!(
        !sb.data().join("dir.db").exists(),
        "the database was created"
    );

    let outside = sb.outside().join("keep.txt");
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let db = sb.data().join(format!("app{}.db", suffix.len()));
        let mut side = db.clone().into_os_string();
        side.push(suffix);
        let side = PathBuf::from(side);
        if !link_file(&side, &outside) {
            // This Windows user can't make file symlinks (no Developer
            // Mode or admin); CI runners and unix can.
            eprintln!("skipping file-symlink cases: no symlink privilege");
            return;
        }
        // The db path itself is checked before the link (for "") exists,
        // as a caller would have done.
        let checked = GuardedPath {
            path: db.clone(),
            root: RootKind::AppData,
        };
        assert!(checked.open_database().is_err(), "{suffix:?} read-write");
        assert!(
            checked.open_database_read_only().is_err(),
            "{suffix:?} read-only"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"keep", "{suffix:?}");
        fs::remove_file(&side).unwrap();
    }
}

/// A symlink to a file. False where the OS won't let this user make one
/// (Windows without Developer Mode or admin rights).
fn link_file(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}
