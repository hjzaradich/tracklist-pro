//! The network gate (0E-6, ROADMAP §0.1): no code outside `src/net/` can
//! make HTTP or raw network calls.
//!
//! Three checks, each also run against a planted violation to prove it
//! catches one:
//!
//! 1. **Source.** No file in `src/` outside `src/net/`, nor `build.rs`,
//!    names an HTTP client or WebSocket crate, a socket API (`std::net`,
//!    `TcpStream`, Windows' networking APIs and DLLs, …), a way to hand a
//!    URL to something that opens it (`ShellExecuteW`, the opener and shell
//!    plugins, `explorer`, `cmd … start`, `WebviewUrl::External`) or a
//!    command-line downloader such as `curl`. Comments count too: rewording
//!    one is cheaper than a gap in the check.
//! 2. **Manifest.** `Cargo.toml` depends on no network crate but `ureq`,
//!    renames none (which would hide it from the source scan), and turns on
//!    no Windows networking or shell APIs.
//! 3. **Dependencies.** In `Cargo.lock`, no crate but the app depends on
//!    `ureq`. Any other HTTP client or socket crate is reached only through
//!    a known mobile-only edge (Tauri's reqwest), and `deny.toml` bans each
//!    of those from the desktop builds, which cargo-deny checks in CI.
//!
//! The webview is covered separately: the CSP blocks remote connections
//! (`tests/csp.rs`), `net::navigation` keeps the window on the app's own
//! pages, and `src/test/noNetworkCalls.test.ts` scans the frontend.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// HTTP client, WebSocket and socket crates, as Cargo names them. Only
/// `ureq`, and only in `src/net/`.
const NETWORK_CRATES: &[&str] = &[
    "ureq",
    "ureq-proto",
    "reqwest",
    "hyper",
    "hyper-util",
    "isahc",
    "curl",
    "curl-sys",
    "attohttpc",
    "minreq",
    "surf",
    "ehttp",
    "http-req",
    "awc",
    "tungstenite",
    "tokio-tungstenite",
    "async-tungstenite",
    "websocket",
    "socket2",
    "tauri-plugin-http",
    "tauri-plugin-websocket",
    "tauri-plugin-upload",
    // Hand a URL to the browser or the shell, which then fetches it.
    "tauri-plugin-opener",
    "tauri-plugin-shell",
    "opener",
    "open",
    "webbrowser",
];

/// Crate names too common as words to scan source for; the manifest and
/// lock checks still catch them, and [`scan_text`] catches their calls.
const COMMON_WORD_CRATES: &[&str] = &["open"];

/// Windows DLLs whose only job is networking or opening URLs, as named in
/// `#[link(name = "…")]` or a `LoadLibrary` call. Matched ignoring case.
const NETWORK_LIBRARIES: &[&str] = &["ws2_32", "mswsock", "winhttp", "wininet", "urlmon"];

/// Programs that fetch or open a URL when launched with one. A string that
/// names one (`"explorer"`, `"C:\Windows\explorer.exe"`) is flagged.
const LAUNCHERS: &[&str] = &[
    "explorer",
    "rundll32",
    "mshta",
    "certutil",
    "powershell",
    "pwsh",
    "bitsadmin",
    "wget",
    "curl",
    "msedge",
    "chrome",
    "firefox",
    "iexplore",
];

/// The one HTTP client, and the app, the only crate allowed to use it.
const HTTP_CLIENT: &str = "ureq";
const APP: &str = "tracklist-pro";

/// Socket and HTTP APIs of the standard library and Windows.
const NETWORK_APIS: &[&str] = &[
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "ToSocketAddrs",
    "WSAStartup",
    "WinHttpOpen",
    "InternetOpenA",
    "InternetOpenW",
    "URLDownloadToFileA",
    "URLDownloadToFileW",
    "WinHttpConnect",
    "WinHttpSendRequest",
    "InternetOpenUrlA",
    "InternetOpenUrlW",
    "HttpOpenRequestA",
    "HttpOpenRequestW",
    // Open a URL in whatever handles it.
    "ShellExecuteA",
    "ShellExecuteW",
    "ShellExecuteExA",
    "ShellExecuteExW",
    "open_url",
];

/// Modules whose `net` submodule opens sockets: `std::net`, `tokio::net`, …
const NET_PARENTS: &[&str] = &["std", "core", "tokio", "async_std", "mio", "smol"];

/// Command-line downloaders the app could shell out to, matched
/// case-insensitively anywhere in the text. (`curl` is caught as a crate
/// name above.)
const DOWNLOADERS: &[&str] = &[
    "wget",
    "invoke-webrequest",
    "invoke-restmethod",
    "start-bitstransfer",
    "bitsadmin",
    "certutil",
    "net.webclient",
    "system.net.http",
];

/// Dependency edges in `Cargo.lock` that reach a network crate without
/// going through the app. Each is mobile-only, and `deny.toml` bans the
/// crate it reaches from desktop builds; see
/// [`lock_exemptions_are_banned_from_desktop_builds`].
const MOBILE_ONLY_EDGES: &[(&str, &str)] = &[
    // Tauri's dev-server proxy, compiled only for Android and iOS.
    ("tauri", "reqwest"),
    ("reqwest", "hyper"),
    ("reqwest", "hyper-util"),
    ("hyper-util", "hyper"),
    ("hyper-util", "socket2"),
    // tokio's `net` feature, which only reqwest turns on.
    ("tokio", "socket2"),
];

// --- 1. Source ---

/// A problem found, as `file:line: what`.
type Finding = String;

/// Every network call in the Rust files under `src` (skipping `src/net/`)
/// and in `extra_files`.
fn scan_source(src: &Path, extra_files: &[PathBuf]) -> Vec<Finding> {
    let net_dir = src.join("net");
    let mut files = Vec::new();
    rust_files(src, &mut files);
    files.retain(|f| !f.starts_with(&net_dir));
    files.extend(extra_files.iter().cloned());
    files.sort();
    let mut findings = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file:?}: {e}"));
        for (line, what) in scan_text(&text) {
            findings.push(format!("{}:{line}: {what}", file.display()));
        }
    }
    findings
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{dir:?}: {e}")) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// A word or a piece of punctuation, with its line number.
#[derive(Debug, PartialEq)]
struct Token<'a> {
    text: &'a str,
    line: usize,
}

/// Splits source into identifiers and the punctuation paths are made of
/// (`::`, `{`, `}`, `,`). Everything else is dropped.
fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let (mut i, mut line) = (0, 1);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push(Token {
                text: &text[start..i],
                line,
            });
        } else if bytes[i..].starts_with(b"::") {
            out.push(Token { text: "::", line });
            i += 2;
        } else if matches!(b, b'{' | b'}' | b',') {
            out.push(Token {
                text: &text[i..i + 1],
                line,
            });
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Every network call in one file's text, as (line, what).
fn scan_text(text: &str) -> Vec<(usize, String)> {
    let crate_idents: Vec<String> = NETWORK_CRATES
        .iter()
        .filter(|c| !COMMON_WORD_CRATES.contains(c))
        .map(|c| c.replace('-', "_"))
        .collect();
    let tokens = tokens(text);
    let mut found = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if crate_idents.iter().any(|c| c == t.text) {
            found.push((t.line, format!("network crate `{}`", t.text)));
        }
        if NETWORK_APIS.contains(&t.text) {
            found.push((t.line, format!("network API `{}`", t.text)));
        }
        if NETWORK_LIBRARIES
            .iter()
            .any(|l| t.text.eq_ignore_ascii_case(l))
        {
            found.push((t.line, format!("network library `{}`", t.text)));
        }
        // `std::net`, `Win32::Networking`, and `std::{io, net}`.
        let next = |k: usize| tokens.get(i + k).map(|t| t.text);
        if next(1) == Some("::") {
            if NET_PARENTS.contains(&t.text) && next(2) == Some("net") {
                found.push((t.line, format!("`{}::net`", t.text)));
            }
            if t.text == "Win32" && next(2).is_some_and(|n| n.starts_with("Network")) {
                found.push((t.line, "Windows networking API".to_owned()));
            }
            if t.text == "Win32" && next(2) == Some("UI") && next(4) == Some("Shell") {
                found.push((t.line, "Windows shell API (opens URLs)".to_owned()));
            }
            // A window showing a remote page.
            if matches!(t.text, "WebviewUrl" | "WindowUrl") && next(2) == Some("External") {
                found.push((t.line, format!("`{}::External`", t.text)));
            }
            // The `open` crate: `open::that(url)`.
            if t.text == "open" && next(2).is_some_and(|n| n.starts_with("that") || n == "with") {
                found.push((t.line, "network crate `open`".to_owned()));
            }
            if NET_PARENTS.contains(&t.text) && next(2) == Some("{") {
                let mut depth = 0;
                for u in &tokens[i + 2..] {
                    match u.text {
                        "{" => depth += 1,
                        "}" => depth -= 1,
                        "net" if depth == 1 => {
                            found.push((u.line, format!("`{}::{{net}}`", t.text)));
                        }
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                }
            }
        }
    }
    let literals = string_literals(text);
    let launched = tokens.iter().any(|t| t.text == "Command");
    // The lines of the strings naming `program`.
    let naming = |program: &str| -> Vec<usize> {
        literals
            .iter()
            .filter(|(_, s)| program_name(s).eq_ignore_ascii_case(program))
            .map(|(line, _)| *line)
            .collect()
    };
    for launcher in LAUNCHERS {
        for line in naming(launcher) {
            found.push((line, format!("launches `{launcher}`")));
        }
    }
    // `cmd /C start <url>` opens a URL; `cmd` alone (e.g. `mklink`) doesn't.
    if !naming("cmd").is_empty() {
        for line in naming("start") {
            found.push((line, "launches `cmd … start`".to_owned()));
        }
    }
    if launched {
        for (line, s) in &literals {
            let lower = s.to_lowercase();
            if ["http://", "https://", "ftp://", "ws://", "wss://"]
                .iter()
                .any(|scheme| lower.starts_with(scheme))
            {
                found.push((*line, "launches a process in a file with a URL".to_owned()));
            }
        }
    }
    for (n, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        for d in DOWNLOADERS {
            if lower.contains(d) {
                found.push((n + 1, format!("downloader `{d}`")));
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// The contents of every `"…"` string in `text`, with the line it starts
/// on. Close enough for a scan: escapes are kept as written, and a string
/// in a comment counts too.
fn string_literals(text: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut line, mut i) = (Vec::new(), 1, 0);
    while i < chars.len() {
        match chars[i] {
            '\n' => line += 1,
            // The char literal `'"'`.
            '"' if i > 0 && chars[i - 1] == '\'' && chars.get(i + 1) == Some(&'\'') => {}
            '"' => {
                let start_line = line;
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        s.push(chars[i]);
                        i += 1;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    s.push(chars[i]);
                    i += 1;
                }
                out.push((start_line, s));
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The program a string names: `explorer` for `C:\Windows\explorer.exe`.
fn program_name(s: &str) -> &str {
    let file = s.trim().rsplit(['/', '\\']).next().unwrap_or("");
    file.strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"))
        .unwrap_or(file)
}

fn src_dir() -> PathBuf {
    Path::new(MANIFEST_DIR).join("src")
}

fn build_rs() -> PathBuf {
    Path::new(MANIFEST_DIR).join("build.rs")
}

#[test]
fn no_code_outside_net_makes_network_calls() {
    let findings = scan_source(&src_dir(), &[build_rs()]);
    assert!(
        findings.is_empty(),
        "Only src/net/ may make network calls (ROADMAP 0.1). Send requests through \
         net::Net instead:\n{}",
        findings.join("\n")
    );
}

#[test]
fn the_scan_reads_the_real_source_and_would_see_the_net_module() {
    // Scanning net/ as if it were any other module finds its HTTP client,
    // so the empty result above isn't a scan that read nothing.
    let net_mod = fs::read_to_string(src_dir().join("net").join("mod.rs")).unwrap();
    let found = scan_text(&net_mod);
    assert!(found.iter().any(|(_, w)| w.contains("`ureq`")), "{found:?}");
    let mut files = Vec::new();
    rust_files(&src_dir(), &mut files);
    assert!(files.contains(&src_dir().join("lib.rs")));
    assert!(files.len() > 10, "only {} files", files.len());
}

#[test]
fn the_scan_catches_planted_violations() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    fs::create_dir_all(src.join("net")).unwrap();
    fs::create_dir_all(src.join("tags")).unwrap();
    fs::create_dir_all(src.join("network")).unwrap();
    // Allowed: the net module itself.
    fs::write(
        src.join("net/mod.rs"),
        "use std::net::TcpStream; use ureq::Agent;",
    )
    .unwrap();
    let planted = [
        ("tags/a.rs", "use std::net::TcpStream;", "`std::net`"),
        (
            "tags/b.rs",
            "use std::{fs, net::UdpSocket};",
            "`std::{net}`",
        ),
        (
            "tags/c.rs",
            "let s = TcpStream::connect(addr);",
            "network API `TcpStream`",
        ),
        ("tags/d.rs", "ureq::get(url).call()", "network crate `ureq`"),
        (
            "tags/e.rs",
            "let c = reqwest::blocking::Client::new();",
            "network crate `reqwest`",
        ),
        ("tags/f.rs", "use tokio::net::TcpListener;", "`tokio::net`"),
        (
            "tags/g.rs",
            "use windows_sys::Win32::Networking::WinSock;",
            "Windows networking API",
        ),
        (
            "tags/h.rs",
            "Command::new(\"curl.exe\").arg(url)",
            "network crate `curl`",
        ),
        (
            "tags/i.rs",
            "Command::new(\"powershell\").arg(\"Invoke-WebRequest\")",
            "downloader `invoke-webrequest`",
        ),
        (
            "tags/j.rs",
            "use tauri_plugin_http::reqwest;",
            "network crate `tauri_plugin_http`",
        ),
        (
            "tags/k.rs",
            "use std::{\n    io,\n    net,\n};",
            "`std::{net}`",
        ),
        // A folder merely starting with "net" isn't exempt.
        ("network/mod.rs", "use std::net::TcpStream;", "`std::net`"),
        // Raw FFI to Windows' networking DLLs.
        (
            "tags/l.rs",
            "#[link(name = \"ws2_32\")] extern \"system\" {}",
            "network library `ws2_32`",
        ),
        (
            "tags/m.rs",
            "#[link(name = \"WinHTTP\", kind = \"raw-dylib\")] extern \"system\" {}",
            "network library `WinHTTP`",
        ),
        (
            "tags/n.rs",
            "let h = LoadLibraryW(w!(\"wininet.dll\"));",
            "network library `wininet`",
        ),
        (
            "tags/o.rs",
            "#[link(name = \"urlmon\")] extern \"system\" {}",
            "network library `urlmon`",
        ),
        // Handing a URL to a program that opens or fetches it.
        (
            "tags/p.rs",
            "Command::new(\"cmd\").args([\"/C\", \"start\", url]).spawn()",
            "launches `cmd … start`",
        ),
        (
            "tags/q.rs",
            "Command::new(\"explorer\").arg(url).spawn()",
            "launches `explorer`",
        ),
        (
            "tags/r.rs",
            "Command::new(\"C:\\\\Windows\\\\explorer.exe\").arg(url)",
            "launches `explorer`",
        ),
        (
            "tags/s.rs",
            "Command::new(\"certutil\").args([\"-urlcache\", \"-f\", url, out])",
            "launches `certutil`",
        ),
        (
            "tags/t.rs",
            "Command::new(program).arg(\"https://example.com/x\")",
            "launches a process in a file with a URL",
        ),
        (
            "tags/u.rs",
            "unsafe { ShellExecuteW(0, w!(\"open\"), url, null(), null(), 1) };",
            "network API `ShellExecuteW`",
        ),
        (
            "tags/v.rs",
            "use windows_sys::Win32::UI::Shell::ShellExecuteExW;",
            "Windows shell API",
        ),
        // A window showing a remote page.
        (
            "tags/w.rs",
            "WebviewWindowBuilder::new(app, \"x\", WebviewUrl::External(url))",
            "`WebviewUrl::External`",
        ),
        // Tauri's opener and shell plugins, and URL-opening crates.
        (
            "tags/x.rs",
            "app.opener().open_url(url, None::<&str>)?;",
            "network API `open_url`",
        ),
        (
            "tags/y.rs",
            "use tauri_plugin_opener::OpenerExt;",
            "network crate `tauri_plugin_opener`",
        ),
        (
            "tags/z.rs",
            "use tauri_plugin_shell::ShellExt; app.shell().open(url, None)",
            "network crate `tauri_plugin_shell`",
        ),
        ("tags/aa.rs", "open::that(url)?;", "network crate `open`"),
        (
            "tags/ab.rs",
            "webbrowser::open(url)?;",
            "network crate `webbrowser`",
        ),
    ];
    for (file, code, _) in &planted {
        fs::write(src.join(file), code).unwrap();
    }
    let extra = root.path().join("build.rs");
    fs::write(
        &extra,
        "fn main() { let _ = std::net::TcpStream::connect(\"x:1\"); }",
    )
    .unwrap();

    let findings = scan_source(&src, std::slice::from_ref(&extra));

    for (file, _, what) in &planted {
        let path: PathBuf = file.split('/').fold(src.clone(), |p, part| p.join(part));
        assert!(
            findings
                .iter()
                .any(|f| f.starts_with(&path.display().to_string()) && f.contains(what)),
            "missed {what} in {file}: {findings:#?}"
        );
    }
    assert!(findings
        .iter()
        .any(|f| f.starts_with(&extra.display().to_string())));
    let net_dir = format!("{}{}", src.join("net").display(), std::path::MAIN_SEPARATOR);
    assert!(
        !findings.iter().any(|f| f.starts_with(&net_dir)),
        "net/ itself is exempt: {findings:#?}"
    );
}

#[test]
fn the_scan_reports_the_line() {
    let found = scan_text("fn a() {}\n\nfn b() { let _ = std::net::UdpSocket::bind(\"x\"); }\n");
    assert!(found.iter().all(|(line, _)| *line == 3), "{found:?}");
    assert_eq!(found.len(), 2, "{found:?}"); // std::net and UdpSocket
}

#[test]
fn ordinary_code_is_not_flagged() {
    let ok = "use std::fs; use crate::net::{Net, Request}; let network = 1; \
              let internet = \"hyperlink\"; fn networking() {} // a curly brace\n\
              let w = Writer::open(&path)?; fn open(p: &Path) {} let opened = true;\n\
              let status = \"started\"; let quote = '\"'; let file = \"explorer.txt\";\n\
              // A test making a junction: cmd, but no `start` and no URL.\n\
              Command::new(\"cmd\").args([\"/C\", \"mklink\", \"/J\"]).arg(&link);";
    assert_eq!(scan_text(ok), vec![]);
}

// --- 2. Manifest ---

/// The names of every dependency in a Cargo.toml, in any dependencies table.
fn manifest_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            let header = line.trim_matches(|c| c == '[' || c == ']');
            // `[dependencies]`, `[target.'cfg(windows)'.dependencies]`, …
            in_deps = header.ends_with("dependencies");
            // `[dependencies.foo]`
            if let Some((table, name)) = header.rsplit_once('.') {
                if table.ends_with("dependencies") {
                    names.insert(name.to_owned());
                }
            }
        } else if in_deps {
            if let Some((name, _)) = line.split_once('=') {
                let name = name.trim().trim_matches('"');
                if !name.is_empty() && !name.starts_with('#') {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    names
}

fn manifest_problems(manifest: &str) -> Vec<String> {
    let mut problems: Vec<String> = manifest_dependencies(manifest)
        .into_iter()
        .filter(|d| NETWORK_CRATES.contains(&d.as_str()) && d != HTTP_CLIENT)
        .map(|d| format!("depends on the network crate `{d}`"))
        .collect();
    if manifest.contains("Win32_Networking") || manifest.contains("Win32_NetworkManagement") {
        problems.push("turns on Windows networking APIs".to_owned());
    }
    if manifest.contains("Win32_UI_Shell") {
        problems.push("turns on the Windows shell API, which opens URLs".to_owned());
    }
    // `alias = { package = "reqwest" }` would hide a crate under another
    // name from the source scan; that goes for ureq too.
    for renamed in package_renames(manifest) {
        if NETWORK_CRATES.contains(&renamed.as_str()) {
            problems.push(format!("renames the network crate `{renamed}`"));
        }
    }
    problems
}

/// Every `package = "…"` in a manifest: the real names of renamed
/// dependencies.
fn package_renames(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = manifest;
    while let Some(at) = rest.find("package") {
        rest = &rest[at + "package".len()..];
        let after = rest.trim_start();
        if let Some(value) = after.strip_prefix('=') {
            if let Some(quoted) = value.trim_start().strip_prefix('"') {
                if let Some(end) = quoted.find('"') {
                    names.push(quoted[..end].to_owned());
                }
            }
        }
    }
    names
}

#[test]
fn the_manifest_has_no_network_crate_but_the_one_client() {
    let manifest = fs::read_to_string(Path::new(MANIFEST_DIR).join("Cargo.toml")).unwrap();
    assert_eq!(manifest_problems(&manifest), Vec::<String>::new());
    assert!(
        manifest_dependencies(&manifest).contains(HTTP_CLIENT),
        "the parser didn't see the net module's client"
    );
}

#[test]
fn the_manifest_check_catches_planted_dependencies() {
    let manifest = r#"
[dependencies]
ureq = "3"
reqwest = { version = "0.12" }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = ["Win32_Networking_WinSock"] }

[dev-dependencies.hyper]
version = "1"

[build-dependencies]
"tauri-plugin-http" = "2"

[dependencies.http]
package = "hyper-util"

[target.'cfg(windows)'.dev-dependencies]
web = { package = "ureq", version = "3" }
tauri-plugin-opener = "2"
open = "5"
windows = { version = "0.61", features = ["Win32_UI_Shell"] }
"#;
    let problems = manifest_problems(manifest);
    for expected in [
        "depends on the network crate `reqwest`",
        "depends on the network crate `hyper`",
        "depends on the network crate `tauri-plugin-http`",
        "Windows networking",
        "renames the network crate `hyper-util`",
        // A renamed ureq would hide from the source scan.
        "renames the network crate `ureq`",
        "depends on the network crate `tauri-plugin-opener`",
        "depends on the network crate `open`",
        "Windows shell API",
    ] {
        assert!(
            problems.iter().any(|p| p.contains(expected)),
            "missed {expected}: {problems:?}"
        );
    }
    assert!(
        !problems
            .iter()
            .any(|p| p.contains("depends on the network crate `ureq`")),
        "{problems:?}"
    );
}

// --- 3. Dependencies ---

/// For each package in a Cargo.lock, the packages it depends on.
fn lock_graph(lock: &str) -> BTreeMap<String, Vec<String>> {
    let mut graph = BTreeMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut name = None;
        let mut deps = Vec::new();
        let mut in_deps = false;
        for line in block.lines().map(str::trim) {
            if let Some(n) = line.strip_prefix("name = ") {
                name = Some(n.trim_matches('"').to_owned());
            } else if line.starts_with("dependencies = [") {
                in_deps = !line.ends_with(']');
            } else if in_deps {
                if line == "]" {
                    in_deps = false;
                } else {
                    // `"name"` or `"name version"` or `"name version (source)"`.
                    let dep = line.trim_end_matches(',').trim_matches('"');
                    deps.push(dep.split(' ').next().unwrap().to_owned());
                }
            }
        }
        let name = name.expect("every package has a name");
        graph.entry(name).or_insert_with(Vec::new).extend(deps);
    }
    graph
}

fn lock_problems(lock: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (package, deps) in lock_graph(lock) {
        for dep in deps {
            if !NETWORK_CRATES.contains(&dep.as_str()) {
                continue;
            }
            let allowed = (package == APP && dep == HTTP_CLIENT)
                // The client's own parts.
                || (package == HTTP_CLIENT && NETWORK_CRATES.contains(&dep.as_str()))
                || MOBILE_ONLY_EDGES.contains(&(package.as_str(), dep.as_str()));
            if !allowed {
                problems.push(format!("`{package}` depends on the network crate `{dep}`"));
            }
        }
    }
    problems
}

#[test]
fn only_the_app_depends_on_the_http_client() {
    let lock = fs::read_to_string(Path::new(MANIFEST_DIR).join("Cargo.lock")).unwrap();
    assert_eq!(
        lock_problems(&lock),
        Vec::<String>::new(),
        "Only the app's net module may make HTTP calls (ROADMAP 0.1)"
    );
    let graph = lock_graph(&lock);
    assert!(
        graph[APP].iter().any(|d| d == HTTP_CLIENT),
        "the parser didn't see the app use ureq"
    );
    assert!(
        graph.len() > 100,
        "the parser read only {} packages",
        graph.len()
    );
}

#[test]
fn the_lock_check_catches_planted_dependents() {
    let lock = r#"
[[package]]
name = "tracklist-pro"
version = "0.1.0"
dependencies = [
 "lofty",
 "ureq",
]

[[package]]
name = "lofty"
version = "0.25.4"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
 "ureq 3.4.2",
]

[[package]]
name = "tauri"
version = "2.12.0"
dependencies = [
 "reqwest",
 "tungstenite 0.24.0 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "ureq"
version = "3.4.2"
dependencies = [
 "native-tls",
 "ureq-proto",
]
"#;
    assert_eq!(
        lock_problems(lock),
        vec![
            "`lofty` depends on the network crate `ureq`".to_owned(),
            "`tauri` depends on the network crate `tungstenite`".to_owned(),
        ]
    );
}

/// The `deny = [...]` entries in deny.toml's `[bans]`, as (crate,
/// wrappers).
fn denied_crates(deny_toml: &str) -> BTreeMap<String, Vec<String>> {
    let mut denied = BTreeMap::new();
    for line in deny_toml.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("{ crate = \"") else {
            continue;
        };
        let name = rest.split('"').next().unwrap().to_owned();
        let wrappers = line
            .split_once("wrappers = [")
            .map(|(_, w)| {
                w.split(']')
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        denied.insert(name, wrappers);
    }
    denied
}

#[test]
fn lock_exemptions_are_banned_from_desktop_builds() {
    // The mobile-only edges are safe only while cargo-deny, which checks the
    // desktop targets, keeps every crate they reach out of the build.
    let deny = fs::read_to_string(Path::new(MANIFEST_DIR).join("../deny.toml")).unwrap();
    let denied = denied_crates(&deny);
    for (_, reached) in MOBILE_ONLY_EDGES {
        assert_eq!(
            denied.get(*reached),
            Some(&Vec::new()),
            "deny.toml must ban `{reached}` outright"
        );
    }
    assert_eq!(
        denied.get(HTTP_CLIENT),
        Some(&vec![APP.to_owned()]),
        "deny.toml must let only the app depend on `{HTTP_CLIENT}`"
    );
    for feature_ban in [
        "crate = \"tokio\"\ndeny = [\"net\"]",
        "crate = \"mio\"\ndeny = [\"net\"]",
    ] {
        assert!(
            deny.replace("\r\n", "\n").contains(feature_ban),
            "deny.toml must ban {feature_ban}"
        );
    }
}
