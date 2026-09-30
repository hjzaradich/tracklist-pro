//! 0E-8, part 1: a source scan that fails if any shipped code outside the
//! write guard can write a file (ROADMAP §5.6, CLAUDE.md "Files and disk").
//!
//! Every `.rs` file under `src-tauri/src` is scanned, except the guard
//! itself (`write_guard/mod.rs`) and test-only code: `#[cfg(test)]` / `#[test]` items and the
//! files of `#[cfg(test)] mod x;` modules (test helpers such as
//! `tags/test_audio.rs` write fixtures into temp dirs). Comments and string
//! contents are ignored, so a pattern mentioned in prose isn't a hit; SQL
//! in strings is checked separately for statements that create files.
//!
//! A hit is a call that can create, change, move or delete a file, open a
//! writable handle, or start a process that could. Route it through
//! [`super::WriteGuard`]. If it truly can't be, add an [`Exemption`] with
//! the reason, and expect the reviewer to ask about it.

use std::path::{Path, PathBuf};

/// Code that can write, with why it's forbidden outside the guard. Each is
/// matched as written, and where it starts or ends with a name, only as a
/// whole name (`remove_dir` doesn't match `remove_dir_all`).
const FORBIDDEN: &[(&str, &str)] = &[
    // std::fs and handles
    ("File::create", "creates or empties a file"),
    ("File::create_new", "creates a file"),
    ("File::options", "can open a writable handle"),
    ("File>::create", "creates or empties a file"),
    ("File>::create_new", "creates a file"),
    ("File>::options", "can open a writable handle"),
    ("Connection>::open", "creates a database file wherever it's pointed"),
    ("Command>::new", "starts a process, which can write anywhere"),
    // raw system-call crates
    ("libc::", "calls the OS directly, around every check here"),
    ("winapi::", "calls the OS directly, around every check here"),
    ("rustix::", "calls the OS directly, around every check here"),
    ("OpenOptions", "can open a writable handle"),
    ("OpenOptionsExt", "can ask Windows for write access"),
    (".write(true)", "opens a writable handle"),
    (".append(true)", "opens a writable handle"),
    (".create(true)", "creates a file"),
    (".create_new(true)", "creates a file"),
    (".truncate(true)", "empties a file"),
    (".access_mode(", "can ask Windows for write access"),
    ("fs::write", "writes a file"),
    ("fs::copy", "writes a file"),
    ("fs::rename", "moves or renames a file"),
    ("remove_file", "deletes a file"),
    ("remove_dir", "deletes a folder"),
    ("remove_dir_all", "deletes a folder tree"),
    ("create_dir", "creates a folder"),
    ("create_dir_all", "creates folders"),
    ("DirBuilder", "creates folders"),
    ("hard_link", "creates a link"),
    ("soft_link", "creates a link"),
    ("symlink", "creates a link"),
    ("symlink_file", "creates a link"),
    ("symlink_dir", "creates a link"),
    ("set_permissions", "changes a file's attributes"),
    ("set_len", "resizes a file"),
    ("set_times", "changes a file's times"),
    ("set_modified", "changes a file's times"),
    ("FileTimes", "changes a file's times"),
    // other processes can write anything
    (
        "process::Command",
        "starts a process, which can write anywhere",
    ),
    ("Command::new", "starts a process, which can write anywhere"),
    // SQLite
    (
        "Connection::open",
        "creates a database file wherever it's pointed",
    ),
    (
        "open_with_flags",
        "opens a database outside the guard; even read-only, SQLite writes -wal/-shm files beside it",
    ),
    (
        "open_with_flags_and_vfs",
        "opens a database outside the guard; even read-only, SQLite writes -wal/-shm files beside it",
    ),
    ("authorizer", "changes the rule that stops SQLite attaching other files"),
    (
        "OpenFlags::default",
        "opens a database read-write, creating it",
    ),
    ("SQLITE_OPEN_READ_WRITE", "opens a database read-write"),
    ("SQLITE_OPEN_CREATE", "creates a database file"),
    ("backup", "writes a database copy"),
    // lofty (tag writing)
    ("save_to", "writes tags into a file"),
    ("save_to_path", "writes tags into a file"),
    ("remove_from", "strips tags from a file"),
    ("remove_from_path", "strips tags from a file"),
    ("write_to", "writes tags into a file"),
    ("write_to_path", "writes tags into a file"),
    ("dump_to", "writes tags into a file"),
    // tauri-specta / specta-typescript
    (".export(", "writes the bindings file"),
    ("export_to", "writes a bindings file"),
    // Win32: opening a handle directly, or adopting a raw one
    ("CreateFileW", "opens a Windows handle directly, maybe writable"),
    ("CreateFileA", "opens a Windows handle directly, maybe writable"),
    ("CreateFile2", "opens a Windows handle directly, maybe writable"),
    ("CreateFileTransactedW", "opens a Windows handle directly, maybe writable"),
    ("ReOpenFile", "reopens a handle, maybe writable"),
    ("OpenFileById", "opens a handle, maybe writable"),
    ("NtCreateFile", "opens a handle, maybe writable"),
    ("ZwCreateFile", "opens a handle, maybe writable"),
    ("NtWriteFile", "writes a file"),
    ("NtSetInformationFile", "renames, deletes or changes a file"),
    ("DeviceIoControl", "can send a device a command that writes"),
    ("from_raw_handle", "adopts a handle opened outside the guard"),
    ("from_raw_fd", "adopts a handle opened outside the guard"),
    // Win32 calls that write
    ("GENERIC_WRITE", "asks Windows for write access"),
    ("GENERIC_ALL", "asks Windows for write access"),
    ("FILE_GENERIC_WRITE", "asks Windows for write access"),
    ("FILE_WRITE_DATA", "asks Windows for write access"),
    ("FILE_APPEND_DATA", "asks Windows for write access"),
    ("FILE_WRITE_ATTRIBUTES", "asks Windows for write access"),
    ("FILE_WRITE_EA", "asks Windows for write access"),
    ("CREATE_ALWAYS", "creates or empties a file"),
    ("CREATE_NEW", "creates a file"),
    ("OPEN_ALWAYS", "creates a file"),
    ("TRUNCATE_EXISTING", "empties a file"),
    ("FILE_FLAG_DELETE_ON_CLOSE", "deletes a file"),
    ("WriteFile", "writes a file"),
    ("WriteFileEx", "writes a file"),
    ("MoveFileW", "moves a file"),
    ("MoveFileExW", "moves a file"),
    ("MoveFileWithProgressW", "moves a file"),
    ("DeleteFileW", "deletes a file"),
    ("CopyFileW", "writes a file"),
    ("CopyFileExW", "writes a file"),
    ("CopyFile2", "writes a file"),
    ("ReplaceFileW", "replaces a file"),
    ("CreateDirectoryW", "creates a folder"),
    ("CreateDirectoryExW", "creates a folder"),
    ("RemoveDirectoryW", "deletes a folder"),
    ("CreateHardLinkW", "creates a link"),
    ("CreateSymbolicLinkW", "creates a link"),
    ("SetFileTime", "changes a file's times"),
    ("SetFileAttributesW", "changes a file's attributes"),
    ("SetEndOfFile", "resizes a file"),
    (
        "SetFileInformationByHandle",
        "renames, deletes or changes a file",
    ),
    ("SHFileOperationW", "moves, copies or deletes files"),
    // The same calls' ANSI (`A`) spellings
    ("MoveFileA", "moves a file"),
    ("MoveFileExA", "moves a file"),
    ("MoveFileWithProgressA", "moves a file"),
    ("DeleteFileA", "deletes a file"),
    ("CopyFileA", "writes a file"),
    ("CopyFileExA", "writes a file"),
    ("ReplaceFileA", "replaces a file"),
    ("CreateDirectoryA", "creates a folder"),
    ("CreateDirectoryExA", "creates a folder"),
    ("RemoveDirectoryA", "deletes a folder"),
    ("CreateHardLinkA", "creates a link"),
    ("CreateSymbolicLinkA", "creates a link"),
    ("SetFileAttributesA", "changes a file's attributes"),
    ("SHFileOperationA", "moves, copies or deletes files"),
    // Transacted, FromApp and 16-bit-era variants
    ("CreateFileTransactedA", "opens a Windows handle directly, maybe writable"),
    ("CreateFileFromAppW", "opens a Windows handle directly, maybe writable"),
    ("CreateFile2FromAppW", "opens a Windows handle directly, maybe writable"),
    ("CopyFileTransactedW", "writes a file"),
    ("CopyFileTransactedA", "writes a file"),
    ("CopyFileFromAppW", "writes a file"),
    ("MoveFileTransactedW", "moves a file"),
    ("MoveFileTransactedA", "moves a file"),
    ("MoveFileFromAppW", "moves a file"),
    ("DeleteFileTransactedW", "deletes a file"),
    ("DeleteFileTransactedA", "deletes a file"),
    ("DeleteFileFromAppW", "deletes a file"),
    ("ReplaceFileFromAppW", "replaces a file"),
    ("CreateDirectoryTransactedW", "creates a folder"),
    ("CreateDirectoryTransactedA", "creates a folder"),
    ("CreateDirectoryFromAppW", "creates a folder"),
    ("RemoveDirectoryTransactedW", "deletes a folder"),
    ("RemoveDirectoryTransactedA", "deletes a folder"),
    ("RemoveDirectoryFromAppW", "deletes a folder"),
    ("CreateHardLinkTransactedW", "creates a link"),
    ("CreateHardLinkTransactedA", "creates a link"),
    ("CreateSymbolicLinkTransactedW", "creates a link"),
    ("CreateSymbolicLinkTransactedA", "creates a link"),
    ("SetFileAttributesTransactedW", "changes a file's attributes"),
    ("SetFileAttributesTransactedA", "changes a file's attributes"),
    ("SetFileAttributesFromAppW", "changes a file's attributes"),
    ("OpenFile", "opens (and can create or delete) a file"),
    ("LZCopy", "writes a file"),
    ("LZOpenFileW", "opens (and can create) a file"),
    ("LZOpenFileA", "opens (and can create) a file"),
    ("_lcreat", "creates a file"),
    ("_lopen", "opens a file, maybe writable"),
    ("_lwrite", "writes a file"),
    ("IFileOperation", "moves, copies or deletes files"),
];

/// `use` of these from `fs` hides the `fs::` prefix the list above matches.
const FS_WRITERS: &[&str] = &["write", "copy", "rename"];

/// Names whose renaming (`use … as`, `type X = …`) would hide a call the
/// list above looks for.
const WATCHED_NAMES: &[&str] = &[
    "fs",
    "File",
    "OpenOptions",
    "DirBuilder",
    "Connection",
    "OpenFlags",
    "process",
    "Command",
];

/// Paths a glob import (`use …::*`) must not come from: it would bring in
/// writing calls under bare names.
const NO_GLOB_FROM: &[&str] = &["fs", "process", "windows_sys", "rusqlite"];

/// The `windows_sys` items shipped code may use: each one reviewed as
/// read-only or query-only. Anything else from `windows_sys` is a hit, so a
/// new Win32 call needs review here even if no pattern above names it.
/// (`CreateFileW` and `DeviceIoControl` are also counted exemptions.)
const WIN32_ALLOWED: &[&str] = &[
    // Foundation
    "CloseHandle",
    "GetLastError",
    "ERROR_MORE_DATA",
    "HANDLE",
    "INVALID_HANDLE_VALUE",
    "MAX_PATH",
    // Storage::FileSystem: volume queries
    "BusType1394",
    "BusTypeMmc",
    "BusTypeSd",
    "BusTypeUsb",
    "CreateFileW",
    "FILE_SHARE_READ",
    "FILE_SHARE_WRITE",
    "FindFirstVolumeW",
    "FindNextVolumeW",
    "FindVolumeClose",
    "GetDriveTypeW",
    "GetVolumeInformationW",
    "GetVolumeNameForVolumeMountPointW",
    "GetVolumePathNameW",
    "GetVolumePathNamesForVolumeNameW",
    "OPEN_EXISTING",
    // System::Ioctl / System::IO: the storage property query
    "DeviceIoControl",
    "IOCTL_STORAGE_QUERY_PROPERTY",
    "PropertyStandardQuery",
    "STORAGE_DEVICE_DESCRIPTOR",
    "STORAGE_PROPERTY_QUERY",
    "StorageDeviceProperty",
    // Storage::FileSystem: a file's id, on the walk's access-0 handle
    "FILE_FLAG_BACKUP_SEMANTICS",
    "FILE_FLAG_OPEN_REPARSE_POINT",
    "FILE_ID_INFO",
    "FILE_SHARE_DELETE",
    "FileIdInfo",
    "GetFileInformationByHandleEx",
    // UI::WindowsAndMessaging and friends: the device watch (volume/devices.rs),
    // a hidden window that only hears WM_DEVICECHANGE. None touches a file.
    "CreateWindowExW",
    "DBT_DEVICEARRIVAL",
    "DBT_DEVICEREMOVECOMPLETE",
    "DBT_DEVTYP_VOLUME",
    "DEV_BROADCAST_HDR",
    "DefWindowProcW",
    "DispatchMessageW",
    "ERROR_CLASS_ALREADY_EXISTS",
    "GetMessageW",
    "GetModuleHandleW",
    "HWND",
    "LPARAM",
    "LRESULT",
    "MSG",
    "RegisterClassW",
    "WM_DEVICECHANGE",
    "WNDCLASSW",
    "WPARAM",
    // System::Threading: fingerprint threads at below-normal priority
    // (1aB-7). Scheduling only; no file access.
    "GetCurrentThread",
    "SetThreadPriority",
    "THREAD_PRIORITY_BELOW_NORMAL",
];

/// SQL that makes SQLite create a file. Matched in string contents and
/// migration files, ignoring case.
const FORBIDDEN_SQL: &[&str] = &[
    "ATTACH DATABASE",
    "ATTACH '",
    "ATTACH \"",
    "ATTACH ?",
    "ATTACH :",
    "VACUUM INTO",
];

/// A reviewed use of a flagged pattern outside the guard: exactly `count`
/// uses of one pattern in one file's shipped code. One more is a hit.
struct Exemption {
    file: &'static str,
    pattern: &'static str,
    count: usize,
    why: &'static str,
}

const EXEMPTIONS: &[Exemption] = &[
    Exemption {
        file: "ipc.rs",
        pattern: ".export(",
        count: 1,
        why: "`export_bindings` regenerates src/bindings.ts for developers. \
              Only examples/export_bindings.rs and tests call it; the app \
              never does (checked below).",
    },
    Exemption {
        file: "db/migrations/mod.rs",
        pattern: "authorizer",
        count: 1,
        why: "the migration runner refuses transaction statements while a \
              migration runs, applying write_guard::sqlite_rule first; when \
              done it reinstalls the guard's rule (install_sqlite_rule).",
    },
    Exemption {
        file: "volume/windows.rs",
        pattern: "CreateFileW",
        count: 2, // the import and the one call
        why: "`storage_device` opens the volume device with access 0 (query \
              only, no read or write) to ask which bus the disk is on.",
    },
    Exemption {
        file: "volume/windows.rs",
        pattern: "DeviceIoControl",
        count: 2, // the import and the one call
        why: "`storage_device` sends IOCTL_STORAGE_QUERY_PROPERTY, a query, \
              on the access-0 handle above.",
    },
    Exemption {
        file: "scan/file_id.rs",
        pattern: "CreateFileW",
        count: 2, // the import and the one call
        why: "`QueryHandle::open` opens each file the walk finds with access \
              0 (query only, no read or write), OPEN_EXISTING, to read its \
              file id.",
    },
];

/// The guard itself: the one file where writes are allowed. Its other
/// files are test-only and skipped as such.
const GUARD_FILE: &str = "write_guard/mod.rs";

/// Folders under `tools/` and whether their code ships in the app. A tool
/// that ships must be scanned too; a new folder must be listed here.
const TOOLS: &[(&str, bool, &str)] = &[(
    "fixture-gen",
    false,
    "a developer tool that writes test fixtures; never bundled",
)];

#[derive(Debug, PartialEq, Eq)]
struct Violation {
    file: String,
    line: usize,
    pattern: String,
    why: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "src/{}:{}: `{}` {}. Route it through crate::write_guard::WriteGuard.",
            self.file, self.line, self.pattern, self.why
        )
    }
}

/// A source file with comments blanked and string contents lifted out.
/// Every character keeps its position, so line numbers still hold.
struct Lexed {
    code: Vec<char>,
    /// Each string literal's contents and where it starts in `code`.
    strings: Vec<(usize, String)>,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn blank(c: char) -> char {
    if c == '\n' {
        '\n'
    } else {
        ' '
    }
}

fn lex(src: &str) -> Lexed {
    let s: Vec<char> = src.chars().collect();
    let at = |i: usize| s.get(i).copied();
    let ident_before = |i: usize| i > 0 && is_ident(s[i - 1]);
    let mut code = Vec::with_capacity(s.len());
    let mut strings = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        // Line comment, including doc comments.
        if c == '/' && at(i + 1) == Some('/') {
            while i < s.len() && s[i] != '\n' {
                code.push(' ');
                i += 1;
            }
            continue;
        }
        // Block comment; they nest.
        if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0;
            while i < s.len() {
                if s[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    code.extend([' ', ' ']);
                    i += 2;
                } else if s[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    code.extend([' ', ' ']);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    code.push(blank(s[i]));
                    i += 1;
                }
            }
            continue;
        }
        // Raw string: r"…", r#"…"#, br#"…"#.
        let raw_start = c == 'r' && (!ident_before(i) || (s[i - 1] == 'b' && !ident_before(i - 1)));
        if raw_start {
            let mut j = i + 1;
            while at(j) == Some('#') {
                j += 1;
            }
            if at(j) == Some('"') {
                let hashes = j - i - 1;
                let start = i;
                code.extend(&s[i..=j]);
                j += 1;
                let mut text = String::new();
                while j < s.len() && !(s[j] == '"' && (1..=hashes).all(|k| at(j + k) == Some('#')))
                {
                    text.push(s[j]);
                    code.push(blank(s[j]));
                    j += 1;
                }
                code.extend(&s[j..(j + 1 + hashes).min(s.len())]);
                strings.push((start, text));
                i = j + 1 + hashes;
                continue;
            }
        }
        // String or byte string.
        if c == '"' {
            let start = i;
            code.push('"');
            let mut j = i + 1;
            let mut text = String::new();
            while j < s.len() && s[j] != '"' {
                if s[j] == '\\' && j + 1 < s.len() {
                    text.push(s[j]);
                    code.push(' ');
                    j += 1;
                }
                text.push(s[j]);
                code.push(blank(s[j]));
                j += 1;
            }
            if j < s.len() {
                code.push('"');
            }
            strings.push((start, text));
            i = j + 1;
            continue;
        }
        // Char literal ('x', '\n', '\u{..}', '"'); otherwise a lifetime.
        if c == '\'' {
            let end = if at(i + 1) == Some('\\') {
                (i + 3..s.len()).find(|&k| s[k] == '\'')
            } else if at(i + 2) == Some('\'') {
                Some(i + 2)
            } else {
                None
            };
            if let Some(end) = end {
                code.extend(s[i..=end].iter().map(|&c| blank(c)));
                i = end + 1;
                continue;
            }
        }
        code.push(c);
        i += 1;
    }
    Lexed { code, strings }
}

/// The index just past the `]` closing the attribute whose `[` is at `open`.
fn attribute_end(code: &[char], open: usize) -> usize {
    let mut depth = 0;
    for (k, &c) in code.iter().enumerate().skip(open) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// Whether an attribute (the text between `#[` and `]`, no spaces) makes
/// its item exist only in test builds.
/// Only `test`, `cfg(test)` and `cfg(all(test, …))` count; anything with
/// `any` or `not` in it can hold in a shipped build.
fn is_test_only(attr: &str) -> bool {
    let tokens: Vec<&str> = attr
        .split(|c: char| !is_ident(c))
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.iter().any(|t| matches!(*t, "any" | "not")) {
        return false;
    }
    attr == "test"
        || attr == "cfg(test)"
        || (attr.starts_with("cfg(all(") && tokens.contains(&"test"))
}

/// Words that start an item (a `fn`, `mod`, `impl`…). Anything else under
/// a test attribute is a field, variant, match arm or statement.
const ITEM_WORDS: &[&str] = &[
    "fn",
    "mod",
    "struct",
    "enum",
    "union",
    "impl",
    "trait",
    "const",
    "static",
    "type",
    "use",
    "extern",
    "unsafe",
    "async",
    "macro_rules",
];

/// Whether the text after a test attribute starts an item.
fn starts_an_item(code: &[char]) -> bool {
    let head: String = code.iter().take(200).collect();
    head.split(|c: char| !is_ident(c))
        .filter(|w| !w.is_empty())
        .find(|w| !matches!(*w, "pub" | "crate" | "super" | "self" | "in"))
        .is_some_and(|w| ITEM_WORDS.contains(&w))
}

/// Where the item starting at `start` ends (exclusive). An item runs to its
/// `;` or through its `{ … }` block. A field, variant, match arm or
/// statement also stops at a `,` or at the `}` closing what holds it, so
/// the code after it is never swallowed.
fn item_end(code: &[char], start: usize) -> usize {
    let item = starts_an_item(&code[start..]);
    let mut depth = 0i32;
    let mut j = start;
    while j < code.len() {
        match code[j] {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ';' if depth == 0 => return j + 1,
            ',' if depth == 0 && !item => return j + 1,
            '}' if depth == 0 && !item => return j,
            '{' if depth == 0 => {
                let mut braces = 0;
                while j < code.len() {
                    match code[j] {
                        '{' => braces += 1,
                        '}' => {
                            braces -= 1;
                            if braces == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                return (j + 1).min(code.len());
            }
            _ => {}
        }
        j += 1;
    }
    code.len()
}

/// Blanks every test-only item in `code` and returns the names of test-only
/// modules declared as `mod name;`, whose files are test-only too. Returns
/// `None` if the whole file is test-only (`#![cfg(test)]`).
fn strip_test_items(code: &mut [char]) -> Option<Vec<String>> {
    let mut test_mods = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if code[i] != '#' {
            i += 1;
            continue;
        }
        let inner = code.get(i + 1) == Some(&'!');
        let open = if inner { i + 2 } else { i + 1 };
        if code.get(open) != Some(&'[') {
            i += 1;
            continue;
        }
        let close = attribute_end(code, open);
        let attr: String = code[open + 1..close.saturating_sub(1)]
            .iter()
            .filter(|c| !c.is_whitespace())
            .collect();
        if !is_test_only(&attr) {
            i = close;
            continue;
        }
        if inner {
            return None;
        }
        // Skip any further attributes, then the item: up to its `;`, or
        // through its `{ … }` block.
        let mut j = close;
        loop {
            while j < code.len() && code[j].is_whitespace() {
                j += 1;
            }
            if code.get(j) == Some(&'#') && code.get(j + 1) == Some(&'[') {
                j = attribute_end(code, j + 1);
            } else {
                break;
            }
        }
        let item_start = j;
        let end = item_end(code, item_start);
        let item: String = code[item_start..end].iter().collect();
        let words: Vec<&str> = item
            .split(|c: char| !is_ident(c))
            .filter(|w| !w.is_empty())
            .collect();
        if item.trim_end().ends_with(';') {
            if let Some(pos) = words.iter().position(|w| *w == "mod") {
                if let Some(name) = words.get(pos + 1) {
                    test_mods.push((*name).to_owned());
                }
            }
        }
        for c in &mut code[i..end] {
            *c = blank(*c);
        }
        i = end;
    }
    Some(test_mods)
}

/// Where `mod name;` in `file` keeps its code: `dir/name.rs` and anything
/// under `dir/name/`.
fn module_paths(file: &str, name: &str) -> (String, String) {
    let (parent, stem) = match file.rsplit_once('/') {
        Some((parent, leaf)) => (format!("{parent}/"), leaf.trim_end_matches(".rs")),
        None => (String::new(), file.trim_end_matches(".rs")),
    };
    let dir = if matches!(stem, "mod" | "lib" | "main") {
        parent
    } else {
        format!("{parent}{stem}/")
    };
    (format!("{dir}{name}.rs"), format!("{dir}{name}/"))
}

/// Every place `pattern` appears in `code` as whole names.
fn find_all(code: &str, pattern: &str) -> Vec<usize> {
    let starts_ident = pattern.chars().next().is_some_and(is_ident);
    let ends_ident = pattern.chars().last().is_some_and(is_ident);
    code.match_indices(pattern)
        .map(|(at, _)| at)
        .filter(|&at| {
            let before = code[..at].chars().next_back();
            let after = code[at + pattern.len()..].chars().next();
            !(starts_ident && before.is_some_and(is_ident))
                && !(ends_ident && after.is_some_and(is_ident))
        })
        .collect()
}

fn line_of(code: &str, byte: usize) -> usize {
    code[..byte].matches('\n').count() + 1
}

/// Scans shipped source files (`(path under src/, contents)`) and returns
/// every write outside the guard that isn't exempt.
fn scan_sources(files: &[(String, String)]) -> Vec<Violation> {
    // First pass: blank test code and learn which files are test modules.
    let mut lexed = Vec::new();
    let mut test_files: Vec<String> = Vec::new();
    let mut test_dirs: Vec<String> = Vec::new();
    for (path, source) in files {
        let mut file = lex(source);
        match strip_test_items(&mut file.code) {
            None => test_files.push(path.clone()),
            Some(mods) => {
                for name in mods {
                    let (file_path, dir) = module_paths(path, &name);
                    test_files.push(file_path);
                    test_dirs.push(dir);
                }
            }
        }
        lexed.push((path, file));
    }

    let mut found = Vec::new();
    for (path, file) in lexed {
        let test_only =
            test_files.contains(path) || test_dirs.iter().any(|d| path.starts_with(d.as_str()));
        if test_only || path == GUARD_FILE {
            continue;
        }
        let code: String = file.code.iter().collect();
        let mut hits = Vec::new();
        let mut hit = |at: usize, pattern: &str, why: &str| {
            hits.push(Violation {
                file: path.clone(),
                line: line_of(&code, at),
                pattern: pattern.to_owned(),
                why: why.to_owned(),
            });
        };
        for (pattern, why) in FORBIDDEN {
            for at in find_all(&code, pattern) {
                hit(at, pattern, why);
            }
        }
        // Imports and aliases that hide a name the list above looks for.
        for at in find_all(&code, "use") {
            let statement = code[at..].split(';').next().unwrap_or_default();
            for (pattern, why) in hidden_by_import(statement) {
                hit(at, &pattern, why);
            }
        }
        for at in find_all(&code, "type") {
            let statement = code[at..].split(';').next().unwrap_or_default();
            if aliases_a_watched_name(statement) {
                hit(
                    at,
                    "type … =",
                    "renames a file-system, database or process item, hiding its calls from this scan",
                );
            }
        }
        // Any other `windows_sys` path, e.g. `windows_sys::…::DeleteFileW(…)`.
        for at in find_all(&code, "windows_sys") {
            // Imports were checked above; this is for paths in code.
            let statement_start = code[..at].rfind([';', '{', '}']).map_or(0, |i| i + 1);
            let in_use = code[statement_start..at]
                .split(|c: char| !is_ident(c))
                .find(|w| !matches!(*w, "" | "pub" | "crate"))
                == Some("use");
            if in_use {
                continue;
            }
            let path: String = code[at..]
                .chars()
                .take_while(|c| is_ident(*c) || *c == ':')
                .collect();
            let item = path.rsplit("::").next().unwrap_or_default();
            if !WIN32_ALLOWED.contains(&item) {
                hit(
                    at,
                    &format!("windows_sys {item}"),
                    "isn't a reviewed read-only Win32 item",
                );
            }
        }
        // `extern { … }` declares OS calls by hand, around everything here.
        for at in find_all(&code, "extern") {
            let rest = code[at + "extern".len()..].trim_start();
            let rest = match rest.strip_prefix('"') {
                Some(abi) => abi.split_once('"').map_or("", |(_, r)| r).trim_start(),
                None => rest,
            };
            if rest.starts_with('{') {
                hit(
                    at,
                    "extern {",
                    "declares OS calls directly, around every check here",
                );
            }
        }
        // Strings inside stripped test code were blanked with it.
        let shipped_strings = file
            .strings
            .iter()
            .filter(|(start, _)| file.code[*start] != ' ');
        for (start, text) in shipped_strings {
            let upper = text.to_uppercase();
            for sql in FORBIDDEN_SQL {
                if upper.contains(sql) {
                    let byte = code.char_indices().nth(*start).map_or(0, |(b, _)| b);
                    hit(byte, sql, "makes SQLite create a database file");
                }
            }
        }
        // A reviewed pattern is allowed exactly as often as it was reviewed.
        for e in EXEMPTIONS.iter().filter(|e| e.file == path.as_str()) {
            let uses = hits.iter().filter(|v| v.pattern == e.pattern).count();
            if uses <= e.count {
                hits.retain(|v| v.pattern != e.pattern);
            }
        }
        found.extend(hits);
    }
    found.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    found
}

/// What a `use` statement hides from the name patterns: a writing call
/// imported bare from `fs`, a rename of a watched name, a glob from a
/// watched path, or a `windows_sys` item that isn't reviewed.
fn hidden_by_import(statement: &str) -> Vec<(String, &'static str)> {
    let names: Vec<&str> = statement
        .split(|c: char| !is_ident(c))
        .filter(|n| !n.is_empty())
        .collect();
    let mut found = Vec::new();
    if names.contains(&"fs") {
        for writer in FS_WRITERS {
            if names.contains(writer) {
                found.push((format!("use fs::{writer}"), "imports a writing call"));
            }
        }
    }
    // `X as Y`, or `x::{self as Y}` where the renamed thing is `x`.
    for (k, name) in names.iter().enumerate() {
        if *name != "as" || k == 0 {
            continue;
        }
        let mut renamed = names[k - 1];
        if renamed == "self" && k >= 2 {
            renamed = names[k - 2];
        }
        if WATCHED_NAMES.contains(&renamed) {
            found.push((
                format!("use {renamed} as"),
                "renames a file-system, database or process item, hiding its calls from this scan",
            ));
        }
    }
    if statement.contains('*') {
        for path in NO_GLOB_FROM {
            if names.contains(path) {
                found.push((
                    format!("use {path}::*"),
                    "imports every name, writing calls included",
                ));
            }
        }
    }
    if names.contains(&"windows_sys") {
        // Items are the names not followed by `::` (those are modules).
        let mut rest = statement;
        while let Some(start) = rest.find(|c: char| is_ident(c)) {
            let tail = &rest[start..];
            let len = tail.find(|c: char| !is_ident(c)).unwrap_or(tail.len());
            let (name, after) = tail.split_at(len);
            let is_module = after.trim_start().starts_with("::");
            let keyword = matches!(name, "use" | "pub" | "crate" | "self" | "super" | "as");
            if !is_module && !keyword && !WIN32_ALLOWED.contains(&name) {
                found.push((
                    format!("windows_sys {name}"),
                    "isn't a reviewed read-only Win32 item",
                ));
            }
            rest = after;
        }
    }
    found
}

/// Whether a `type X = …` alias names a watched item directly (a
/// `Box<dyn FnOnce(&mut Connection)>` merely mentions one, and is fine).
fn aliases_a_watched_name(statement: &str) -> bool {
    let Some((_, target)) = statement.split_once('=') else {
        return false;
    };
    let target = target.trim();
    if target.contains(['<', '(', '&', '[']) {
        return false;
    }
    let last = target.rsplit("::").next().unwrap_or_default().trim();
    WATCHED_NAMES.contains(&last)
}

// ---- the real scan -------------------------------------------------------

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, base, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let rel = path.strip_prefix(base).unwrap().to_string_lossy();
            let rel = rel.replace('\\', "/");
            out.push((rel, std::fs::read_to_string(&path).unwrap()));
        }
    }
}

fn shipped_sources() -> Vec<(String, String)> {
    let src = manifest_dir().join("src");
    let mut files = Vec::new();
    rust_files(&src, &src, &mut files);
    files.sort();
    files
}

#[test]
fn no_shipped_code_outside_the_write_guard_can_write_a_file() {
    let files = shipped_sources();
    assert!(
        files.iter().any(|(p, _)| p == "lib.rs") && files.len() > 10,
        "the scan didn't find the app's sources: {:?}",
        files.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    let found = scan_sources(&files);
    let report: Vec<String> = found.iter().map(ToString::to_string).collect();
    assert!(
        found.is_empty(),
        "code outside the write guard can write:\n{}",
        report.join("\n")
    );
}

#[test]
fn the_scan_covers_the_guard_protected_modules_and_skips_test_helpers() {
    // A guard against the scan quietly scanning nothing: these app files
    // are read, and these test helpers are recognised as test-only.
    let files = shipped_sources();
    let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    for app in [
        "lib.rs",
        "db/writer.rs",
        "db/read_pool.rs",
        "paths/mod.rs",
        "tags/mod.rs",
        "volume/mod.rs",
    ] {
        assert!(names.contains(&app), "{app} not scanned");
    }
    let mut test_files = Vec::new();
    for (path, source) in &files {
        let mut code = lex(source).code;
        for name in strip_test_items(&mut code).unwrap_or_default() {
            test_files.push(module_paths(path, &name).0);
        }
    }
    for helper in ["tags/test_audio.rs", "tags/tests.rs"] {
        assert!(
            test_files.iter().any(|f| f == helper),
            "{helper} isn't seen as test-only: {test_files:?}"
        );
    }
}

#[test]
fn every_exemption_matches_its_reviewed_count_exactly() {
    // An exemption whose uses dropped would silently allow new ones back
    // up to its count; one whose uses grew fails the main scan.
    for e in EXEMPTIONS {
        let (_, source) = shipped_sources()
            .into_iter()
            .find(|(p, _)| p == e.file)
            .unwrap_or_else(|| panic!("exempt file {} is gone", e.file));
        let mut code = lex(&source).code;
        strip_test_items(&mut code);
        let code: String = code.into_iter().collect();
        let uses = find_all(&code, e.pattern).len();
        assert_eq!(
            uses, e.count,
            "{}: `{}` is used {uses} times, reviewed for {}; update or remove the exemption ({})",
            e.file, e.pattern, e.count, e.why
        );
    }
}

#[test]
fn the_app_never_calls_the_bindings_export() {
    // Its exemption depends on this: only tests and examples/ call it.
    for (path, source) in shipped_sources() {
        if path == "ipc.rs" {
            continue;
        }
        let mut code = lex(&source).code;
        strip_test_items(&mut code);
        let code: String = code.into_iter().collect();
        assert!(
            find_all(&code, "export_bindings").is_empty(),
            "{path} calls ipc::export_bindings"
        );
    }
    let ipc = std::fs::read_to_string(manifest_dir().join("src").join("ipc.rs")).unwrap();
    let mut code = lex(&ipc).code;
    strip_test_items(&mut code);
    let code: String = code.into_iter().collect();
    assert_eq!(
        find_all(&code, "export_bindings").len(),
        1,
        "ipc.rs should only define export_bindings, not call it"
    );
}

#[test]
fn migrations_do_not_attach_or_vacuum_into_other_files() {
    let dir = manifest_dir().join("migrations");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "sql") {
            let sql = std::fs::read_to_string(&path).unwrap().to_uppercase();
            for forbidden in FORBIDDEN_SQL {
                assert!(!sql.contains(forbidden), "{path:?} contains {forbidden}");
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "no migrations found in {dir:?}");
}

#[test]
fn every_tools_folder_is_known_to_ship_or_not() {
    let tools = manifest_dir().parent().unwrap().join("tools");
    for entry in std::fs::read_dir(&tools).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let known = TOOLS.iter().find(|(tool, _, _)| *tool == name);
        let Some((_, ships, _)) = known else {
            panic!("tools/{name} is new: add it to TOOLS in write_guard/scan.rs");
        };
        assert!(
            !ships,
            "tools/{name} ships: extend the write scan to cover its sources"
        );
    }
}

#[test]
fn no_plugin_lets_the_frontend_write_files_or_run_programs() {
    // The webview can't reach the guard, so it must have no file-writing
    // or process plugin of its own.
    let root = manifest_dir().parent().unwrap().to_path_buf();
    let cargo = std::fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();
    let npm = std::fs::read_to_string(root.join("package.json")).unwrap();
    for plugin in ["tauri-plugin-fs", "tauri-plugin-shell"] {
        assert!(!cargo.contains(plugin), "Cargo.toml adds {plugin}");
    }
    for plugin in ["@tauri-apps/plugin-fs", "@tauri-apps/plugin-shell"] {
        assert!(!npm.contains(plugin), "package.json adds {plugin}");
    }
}

// ---- the scanner itself --------------------------------------------------

fn scan_one(path: &str, source: &str) -> Vec<Violation> {
    scan_sources(&[(path.to_owned(), source.to_owned())])
}

fn patterns(found: &[Violation]) -> Vec<&str> {
    found.iter().map(|v| v.pattern.as_str()).collect()
}

#[test]
fn the_scanner_catches_every_forbidden_call_on_its_own() {
    for (pattern, _) in FORBIDDEN {
        let source = format!("fn f() {{\n    a {pattern} b;\n}}\n");
        let found = scan_one("x.rs", &source);
        assert_eq!(patterns(&found), [*pattern], "in {source:?}");
        assert_eq!(found[0].line, 2);
    }
}

#[test]
fn the_scanner_catches_a_planted_write_the_way_it_would_be_written() {
    let source = r#"
use std::io::Write;

pub fn save(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

pub fn log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::File::options()
        .append(true)
        .open(path)
}
"#;
    let found = scan_one("library/save.rs", source);
    let lines: Vec<(usize, &str)> = found.iter().map(|v| (v.line, v.pattern.as_str())).collect();
    assert_eq!(
        lines,
        [
            (5, "fs::write"),
            (9, "File::options"),
            (10, ".append(true)")
        ]
    );
    let message = found[0].to_string();
    assert!(
        message.starts_with("src/library/save.rs:5: `fs::write`"),
        "{message}"
    );
}

#[test]
fn the_scanner_catches_an_import_that_hides_the_fs_prefix() {
    let found = scan_one(
        "x.rs",
        "use std::fs::{self, write};\nfn f() { write(p, b); }\n",
    );
    assert_eq!(patterns(&found), ["use fs::write"]);
    assert!(scan_one("x.rs", "use std::io::Write;\nuse std::fs::File;\n").is_empty());
}

#[test]
fn the_scanner_skips_test_code_but_not_the_code_after_it() {
    let source = r#"
#[cfg(test)]
mod tests {
    fn fixture() { std::fs::write("a", "b").unwrap(); }
    #[test]
    fn t() { let _ = std::fs::File::create("x"); }
}

#[cfg(test)]
fn helper() { std::fs::remove_file("x").unwrap(); }

#[cfg(all(test, windows))]
#[allow(dead_code)]
const JUNK: [u8; 2] = { std::fs::create_dir("x"); [0; 2] };

#[test]
fn stray_test() { std::fs::rename("a", "b").unwrap(); }

#[cfg_attr(not(test), allow(dead_code))]
fn app() { std::fs::copy("a", "b").unwrap(); }

#[cfg(not(test))]
fn app2() { std::fs::write("a", "b").unwrap(); }

#[cfg(any(test, feature = "x"))]
fn app3() { std::fs::remove_dir("a").unwrap(); }
"#;
    let found = scan_one("x.rs", source);
    assert_eq!(patterns(&found), ["fs::copy", "fs::write", "remove_dir"]);
}

#[test]
fn the_scanner_ignores_comments_strings_and_char_literals() {
    let source = r##"
// std::fs::write in a comment
/// Doc: never call `File::create` here.
/* block /* nested fs::rename */ still comment remove_file */
fn f<'a>(x: &'a str) -> char {
    let a = "fs::write { not code";
    let b = r#"File::create "quoted" } "#;
    let c = b"remove_file";
    let d = '{';
    let e = '"';
    let g = '\'';
    let h = '\u{7b}';
    'x'
}
fn after() { std::fs::copy("a", "b").unwrap(); }
"##;
    let found = scan_one("x.rs", source);
    assert_eq!(patterns(&found), ["fs::copy"]);
    assert_eq!(found[0].line, 15);
}

#[test]
fn a_test_module_file_is_skipped_and_its_siblings_are_not() {
    let files = [
        (
            "tags/mod.rs".to_owned(),
            "#[cfg(test)]\npub(crate) mod test_audio;\nmod raw;\n".to_owned(),
        ),
        (
            "tags/test_audio.rs".to_owned(),
            "fn f() { std::fs::write(p, b); }".to_owned(),
        ),
        (
            "tags/test_audio/more.rs".to_owned(),
            "fn f() { std::fs::write(p, b); }".to_owned(),
        ),
        (
            "tags/raw.rs".to_owned(),
            "fn f() { std::fs::write(p, b); }".to_owned(),
        ),
        (
            "db/writer.rs".to_owned(),
            "#[cfg(test)]\nmod fixtures;\nfn f() {}\n".to_owned(),
        ),
        (
            "db/writer/fixtures.rs".to_owned(),
            "fn f() { std::fs::write(p, b); }".to_owned(),
        ),
        (
            "db/fixtures.rs".to_owned(),
            "fn f() { std::fs::write(p, b); }".to_owned(),
        ),
        (
            "whole.rs".to_owned(),
            "#![cfg(test)]\nfn f() { std::fs::write(p, b); }".to_owned(),
        ),
    ];
    let found = scan_sources(&files);
    let where_: Vec<&str> = found.iter().map(|v| v.file.as_str()).collect();
    assert_eq!(where_, ["db/fixtures.rs", "tags/raw.rs"]);
}

#[test]
fn the_guard_file_and_listed_exemptions_are_the_only_exceptions() {
    let write = "fn f() { std::fs::write(p, b); }";
    assert!(scan_one("write_guard/mod.rs", write).is_empty());
    // Only the guard's own file: a new shipped file beside it is scanned.
    assert!(!scan_one("write_guard/helper.rs", write).is_empty());
    assert!(!scan_one("write_guard_evil.rs", write).is_empty());
    let export = "fn f() { builder.export(ts, path); }";
    assert!(scan_one("ipc.rs", export).is_empty());
    assert_eq!(patterns(&scan_one("other.rs", export)), [".export("]);
    // The exemption covers only its own pattern.
    assert_eq!(patterns(&scan_one("ipc.rs", write)), ["fs::write"]);
}

#[test]
fn the_walks_file_id_exemption_allows_its_one_open_and_no_second() {
    // The shipped file, as reviewed, passes.
    let shipped = include_str!("../scan/file_id.rs");
    assert!(
        scan_one("scan/file_id.rs", shipped).is_empty(),
        "{:?}",
        scan_one("scan/file_id.rs", shipped)
    );
    // One more open in that file is a hit, however it asks.
    for extra in [
        "fn g() { unsafe { CreateFileW(p, 0, 0, null(), OPEN_EXISTING, 0, null_mut()) }; }\n",
        "fn g() { unsafe { CreateFileW(p, GENERIC_READ, 0, null(), OPEN_EXISTING, 0, h) }; }\n",
    ] {
        let more = format!("{shipped}{extra}");
        assert!(
            patterns(&scan_one("scan/file_id.rs", &more)).contains(&"CreateFileW"),
            "{extra}"
        );
    }
    // The exemption is for that file only.
    assert!(patterns(&scan_one("scan/walk.rs", shipped)).contains(&"CreateFileW"));
}

#[test]
fn an_exemption_allows_only_its_reviewed_number_of_uses() {
    let reviewed = "use x::{CreateFileW, DeviceIoControl};
                    fn f() { CreateFileW(p, 0); DeviceIoControl(h); }
";
    assert!(scan_one("volume/windows.rs", reviewed).is_empty());
    // One more open, even in the exempt file, is a hit (all three shown).
    let more = format!(
        "{reviewed}fn g() {{ CreateFileW(p, GENERIC_READ); }}
"
    );
    assert_eq!(
        patterns(&scan_one("volume/windows.rs", &more)),
        ["CreateFileW", "CreateFileW", "CreateFileW"]
    );
    // Elsewhere, any use is a hit.
    assert_eq!(
        patterns(&scan_one("volume/mod.rs", reviewed)),
        [
            "CreateFileW",
            "DeviceIoControl",
            "CreateFileW",
            "DeviceIoControl"
        ]
    );
}

#[test]
fn a_test_attribute_on_a_field_variant_or_arm_hides_only_that_part() {
    // Review finding: the scanner skipped to the next `{`, hiding the
    // shipped function after a test-only field.
    let source = r#"
struct S {
    #[cfg(test)]
    probe: u8,
    kept: u8,
}
fn after_field() { std::fs::write("a", "b").unwrap(); }

enum E {
    #[cfg(test)]
    Probe(u8),
    #[cfg(test)]
    Other { x: u8 },
    Kept,
}
fn after_variant() { std::fs::copy("a", "b").unwrap(); }

fn f(e: E) {
    match e {
        #[cfg(test)]
        E::Probe(_) => {}
        E::Kept => std::fs::remove_file("a").unwrap(),
        _ => {}
    }
    #[cfg(test)]
    let _probe = std::fs::rename("a", "b");
    std::fs::create_dir("a").unwrap();
}

#[cfg(test)]
fn generic<A, B>(a: A, b: B) { std::fs::write("t", "t").unwrap(); }
"#;
    assert_eq!(
        patterns(&scan_one("x.rs", source)),
        ["fs::write", "fs::copy", "remove_file", "create_dir"]
    );
}

#[test]
fn only_plain_test_cfgs_count_as_test_only() {
    let body = "fn f() { std::fs::write(p, b); }";
    for shipped in [
        "#[cfg(all(any(test, not(debug_assertions))))]",
        "#[cfg(any(test, windows))]",
        "#[cfg(all(test, not(windows)))]",
        "#[cfg(not(test))]",
        "#[cfg_attr(test, allow(dead_code))]",
    ] {
        let found = scan_one(
            "x.rs",
            &format!(
                "{shipped}
{body}
"
            ),
        );
        assert_eq!(patterns(&found), ["fs::write"], "{shipped}");
    }
    for test_only in ["#[test]", "#[cfg(test)]", "#[cfg(all(test, windows))]"] {
        let found = scan_one(
            "x.rs",
            &format!(
                "{test_only}
{body}
"
            ),
        );
        assert!(found.is_empty(), "{test_only}: {found:?}");
    }
}

#[test]
fn renaming_a_file_system_database_or_process_item_is_caught() {
    let renamed = |p: &&str| (p.starts_with("use ") && p.ends_with(" as")) || *p == "type … =";
    for source in [
        "use std::fs::File as F;\nfn f() { F::create(p); }",
        "use std::fs as f;\nfn f() { f::write(p, b); }",
        "use std::{fs as f, io};\nfn f() { f::write(p, b); }",
        "use std::fs::{self as f};\nfn f() { f::write(p, b); }",
        "type F = std::fs::File;\nfn f() { F::create(p); }",
        // Review round 3: database and process items too.
        "use rusqlite::Connection as Db;\nfn f() { Db::open(p); }",
        "use std::process::{Command as C};\nfn f() { C::new(\"cmd\"); }",
        "use std::process as p;\nfn f() { p::Command::new(\"cmd\"); }",
        "type Db = rusqlite::Connection;\nfn f() { Db::open(p); }",
    ] {
        let found = scan_one("x.rs", source);
        assert!(patterns(&found).iter().any(renamed), "{source}: {found:?}");
    }
    for (source, pattern) in [
        ("fn f() { <std::fs::File>::create(p); }", "File>::create"),
        (
            "fn f() { <rusqlite::Connection>::open(p); }",
            "Connection>::open",
        ),
        (
            "fn f() { <std::process::Command>::new(p); }",
            "Command>::new",
        ),
    ] {
        let found = scan_one("x.rs", source);
        assert!(patterns(&found).contains(&pattern), "{source}: {found:?}");
    }
    // Plain imports, unrelated renames, `as _` trait imports and aliases
    // that merely mention a watched type are fine.
    for fine in [
        "use std::fs::File;\nuse std::io::Result as R;",
        "use rusqlite::{Connection, OptionalExtension as _};",
        "type Job = Box<dyn FnOnce(&mut Connection) + Send>;",
    ] {
        let found = scan_one("x.rs", fine);
        assert!(found.is_empty(), "{fine}: {found:?}");
    }
}

#[test]
fn a_glob_import_from_a_writing_module_is_caught() {
    for (source, pattern) in [
        ("use std::fs::*;\nfn f() { write(p, b); }", "use fs::*"),
        ("use std::process::*;", "use process::*"),
        ("use rusqlite::*;", "use rusqlite::*"),
        (
            "use windows_sys::Win32::Storage::FileSystem::*;",
            "use windows_sys::*",
        ),
    ] {
        let found = scan_one("x.rs", source);
        assert!(patterns(&found).contains(&pattern), "{source}: {found:?}");
    }
    // Globs elsewhere are fine (`lofty::prelude::*` is used today).
    assert!(scan_one("x.rs", "use lofty::prelude::*;").is_empty());
}

#[test]
fn only_reviewed_windows_sys_items_may_be_used() {
    // Reviewed items, as imported today.
    let reviewed = "use windows_sys::Win32::Foundation::{GetLastError, MAX_PATH};\n\
                    use windows_sys::Win32::Storage::FileSystem::{FindFirstVolumeW, FindVolumeClose};";
    assert!(scan_one("paths/system.rs", reviewed).is_empty());
    // An item no pattern names still needs review, imported or inline.
    for (source, pattern) in [
        (
            "use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, SetFileValidData};",
            "windows_sys SetFileValidData",
        ),
        (
            "fn f() { unsafe { windows_sys::Win32::Storage::FileSystem::SetFileShortNameW(h, n) }; }",
            "windows_sys SetFileShortNameW",
        ),
    ] {
        let found = scan_one("x.rs", source);
        assert!(patterns(&found).contains(&pattern), "{source}: {found:?}");
    }
}

#[test]
fn the_newer_win32_spellings_and_hand_declared_os_calls_are_caught() {
    for name in [
        "CreateFileTransactedW",
        "CopyFileFromAppW",
        "DeleteFileTransactedW",
        "MoveFileFromAppW",
        "OpenFile",
        "LZCopy",
        "_lcreat",
    ] {
        let found = scan_one("x.rs", &format!("fn f() {{ {name}(p); }}"));
        assert_eq!(patterns(&found), [name]);
    }
    for block in [
        "extern \"system\" { fn DeleteFileW(p: *const u16) -> i32; }",
        "extern \"C\" {\n    fn unlink(p: *const i8) -> i32;\n}",
        "extern { fn unlink(p: *const i8) -> i32; }",
    ] {
        assert!(
            patterns(&scan_one("x.rs", block)).contains(&"extern {"),
            "{block}"
        );
    }
    // An `extern "C" fn` callback defined here declares nothing.
    assert!(scan_one("x.rs", "unsafe extern \"C\" fn cb(x: i32) -> i32 { x }").is_empty());
    for raw in [
        "libc::unlink(p)",
        "winapi::um::fileapi::DeleteFileW(p)",
        "rustix::fs::unlink(p)",
    ] {
        let found = scan_one("x.rs", &format!("fn f() {{ {raw}; }}"));
        assert!(!found.is_empty(), "{raw}");
    }
}

#[test]
fn sql_in_test_code_is_not_flagged() {
    let source = "#[cfg(test)]
mod tests {
    fn t() { c.execute_batch(\"VACUUM INTO 'x.db'\"); }
}
";
    assert!(scan_one("x.rs", source).is_empty());
}

#[test]
fn the_scanner_flags_sql_that_creates_database_files() {
    let found = scan_one(
        "x.rs",
        "fn f() {\n    c.execute(\"attach database ?1 as other\", []);\n    c.execute(\"VACUUM INTO 'x.db'\", []);\n}\n",
    );
    assert_eq!(patterns(&found), ["ATTACH DATABASE", "VACUUM INTO"]);
    assert_eq!(found[0].line, 2);
    let prose = "fn f() { let s = \"attach the file to your report\"; }";
    assert!(scan_one("x.rs", prose).is_empty());
}

#[test]
fn names_match_whole_not_as_parts_of_longer_names() {
    assert!(scan_one("x.rs", "fn f() { my_remove_file_helper(); save_total(); }").is_empty());
    assert_eq!(
        patterns(&scan_one("x.rs", "fn f() { std::fs::remove_dir_all(p); }")),
        ["remove_dir_all"]
    );
    assert!(scan_one("x.rs", "fn f() { Connection::open_in_memory(); }").is_empty());
}

#[test]
fn a_windows_sys_module_alias_stays_caught() {
    // 1aA-14. Renaming a `windows_sys` module (or the crate) hides the path
    // the inline check reads, so the import itself must be a hit, and a
    // listed call must still be caught under its new prefix.
    let call = "fn f() { unsafe { fs::DeleteFileW(p) }; }";
    for alias in [
        "use windows_sys::Win32::Storage::FileSystem as fs;",
        "use windows_sys::Win32::Storage::FileSystem::{self as fs};",
        "pub(crate) use windows_sys::Win32::Storage::FileSystem as fs;",
        "use windows_sys::Win32::{Foundation::GetLastError, Storage::FileSystem as fs};",
        "use windows_sys::Win32::Storage as fs;",
        "use windows_sys as fs;",
        "extern crate windows_sys as fs;",
    ] {
        let found = scan_one("x.rs", &format!("{alias}\n{call}\n"));
        let on_line = |line: usize| found.iter().filter(|v| v.line == line).collect::<Vec<_>>();
        assert!(
            on_line(1)
                .iter()
                .any(|v| v.pattern.starts_with("windows_sys ")),
            "{alias}: the alias isn't a hit: {found:?}"
        );
        assert!(
            on_line(2).iter().any(|v| v.pattern == "DeleteFileW"),
            "{alias}: the call isn't a hit: {found:?}"
        );
    }
    // A call no pattern names is still stopped at the alias.
    let found = scan_one(
        "x.rs",
        "use windows_sys::Win32::Storage::FileSystem as fs;\n\
         fn f() { unsafe { fs::SetFileValidData(h, n) }; }\n",
    );
    assert!(
        found
            .iter()
            .any(|v| v.line == 1 && v.pattern.starts_with("windows_sys ")),
        "{found:?}"
    );
}
