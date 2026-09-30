//! The window's Content Security Policy (0C-10) stays strict: the app's own
//! files only, plus what Tauri's IPC needs. No remote origins, no `eval`.
//!
//! The policy is read through Tauri's own config types, so the test checks
//! the exact header text Tauri sends with every page. Tauri adds hashes for
//! its own scripts and styles to that header when it builds the app; those
//! aren't in the config and aren't checked here.

use tauri::utils::config::{Config, Csp, DisabledCspModificationKind};

fn config() -> Config {
    serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses")
}

/// A policy taken apart, in order: (directive name, sources), lowercased.
/// A list, not a map, so a directive given twice is still seen twice.
fn directives(policy: &str) -> Vec<(String, Vec<String>)> {
    policy
        .split(';')
        .filter_map(|d| {
            let mut tokens = d.split_whitespace().map(str::to_lowercase);
            Some((tokens.next()?, tokens.collect()))
        })
        .collect()
}

/// The sources of a directive as a browser enforces it: the first time it
/// appears.
fn sources<'a>(directives: &'a [(String, Vec<String>)], name: &str) -> Option<&'a [String]> {
    directives
        .iter()
        .find(|(d, _)| d == name)
        .map(|(_, s)| s.as_slice())
}

/// What each directive may contain beyond `'self'` and `'none'`. Tauri's
/// IPC calls go to `ipc://localhost`, which WebView2 on Windows spells
/// `http://ipc.localhost`. Nothing else is allowed anywhere.
fn extra_sources_allowed(directive: &str) -> &'static [&'static str] {
    match directive {
        "connect-src" => &["ipc:", "http://ipc.localhost"],
        _ => &[],
    }
}

/// Directives that must be `'none'`. `form-action`, `frame-ancestors` and
/// `base-uri` don't fall back to `default-src`, so leaving them out would
/// leave them wide open.
const MUST_BE_NONE: [&str; 5] = [
    "object-src",
    "base-uri",
    "form-action",
    "frame-src",
    "frame-ancestors",
];

/// Every way `policy` is looser than the app allows. Empty means strict.
fn problems(policy: Option<&Csp>) -> Vec<String> {
    let Some(policy) = policy else {
        return vec!["there is no CSP (csp is null)".into()];
    };
    let policy = policy.to_string();
    let directives = directives(&policy);
    let mut out = Vec::new();

    for (i, (name, _)) in directives.iter().enumerate() {
        if directives[..i].iter().any(|(earlier, _)| earlier == name) {
            out.push(format!(
                "{name} appears twice; browsers enforce only the first"
            ));
        }
    }
    match sources(&directives, "default-src") {
        Some([only]) if only == "'self'" || only == "'none'" => {}
        other => out.push(format!("default-src must be 'self' alone, not {other:?}")),
    }
    for directive in MUST_BE_NONE {
        if !matches!(sources(&directives, directive), Some([only]) if only == "'none'") {
            out.push(format!("{directive} must be 'none'"));
        }
    }
    for (directive, sources) in &directives {
        let extra = extra_sources_allowed(directive);
        for source in sources {
            let allowed =
                source == "'self'" || source == "'none'" || extra.contains(&source.as_str());
            if !allowed {
                out.push(format!("{directive} allows {source}"));
            }
        }
    }
    out
}

fn problems_in(policy: &str) -> Vec<String> {
    problems(Some(&Csp::Policy(policy.into())))
}

const STRICT: &str = "default-src 'self'; object-src 'none'; base-uri 'none'; \
                      form-action 'none'; frame-src 'none'; frame-ancestors 'none'; \
                      connect-src 'self' ipc: http://ipc.localhost";

/// [`STRICT`] with one directive set to `value` (replaced, or added), so a
/// test changes exactly one thing.
fn strict_with(directive: &str, value: &str) -> String {
    let mut parts: Vec<String> = STRICT
        .split(';')
        .map(|d| d.trim().to_owned())
        .filter(|d| d.split_whitespace().next() != Some(directive))
        .collect();
    parts.push(format!("{directive} {value}"));
    parts.join("; ")
}

/// [`STRICT`] without one directive.
fn strict_without(directive: &str) -> String {
    STRICT
        .split(';')
        .map(str::trim)
        .filter(|d| d.split_whitespace().next() != Some(directive))
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn the_app_has_a_strict_csp() {
    let config = config();
    let found = problems(config.app.security.csp.as_ref());
    assert!(found.is_empty(), "the CSP is too loose: {found:#?}");
}

#[test]
fn the_dev_csp_if_one_is_set_is_just_as_strict() {
    // Unset, Tauri uses the main CSP in dev too. (On Windows the dev
    // window loads the Vite server directly, and Tauri injects no CSP
    // there at all; the built app is what this protects.)
    let config = config();
    if let Some(dev) = &config.app.security.dev_csp {
        let found = problems(Some(dev));
        assert!(found.is_empty(), "the dev CSP is too loose: {found:#?}");
    }
}

#[test]
fn tauri_is_still_allowed_to_pin_its_own_scripts_and_styles() {
    // With this switched on, Tauri stops adding hashes for its bundled
    // scripts and styles, and a looser policy would be needed.
    let config = config();
    assert!(
        matches!(
            config.app.security.dangerous_disable_asset_csp_modification,
            DisabledCspModificationKind::Flag(false)
        ),
        "dangerousDisableAssetCspModification must stay off"
    );
}

#[test]
fn the_app_csp_allows_tauri_ipc() {
    let config = config();
    let policy = config.app.security.csp.expect("a CSP").to_string();
    let directives = directives(&policy);
    let connect = sources(&directives, "connect-src").expect("a connect-src");
    for source in ["ipc:", "http://ipc.localhost"] {
        assert!(
            connect.iter().any(|s| s == source),
            "connect-src lacks {source}"
        );
    }
}

// The checks above are only as good as `problems`, so it's tested too.

#[test]
fn the_check_accepts_a_strict_policy() {
    assert_eq!(problems_in(STRICT), Vec::<String>::new());
}

#[test]
fn the_check_rejects_a_null_csp() {
    assert!(!problems(None).is_empty());
}

/// Asserts `policy` has exactly one problem, and that it mentions `what`.
fn assert_one_problem(policy: &str, what: &str) {
    let found = problems_in(policy);
    assert!(
        found.len() == 1 && found[0].contains(what),
        "{policy}\nexpected one problem about {what:?}, got {found:#?}"
    );
}

#[test]
fn the_check_rejects_wildcards() {
    // Both "must be 'self'" and "allows *".
    let found = problems_in(&strict_with("default-src", "*"));
    assert!(found.iter().any(|p| p.contains("allows *")), "{found:#?}");
    assert_one_problem(&strict_with("img-src", "*"), "allows *");
    assert_one_problem(
        &strict_with("connect-src", "'self' https://*.example.com"),
        "allows https://*.example.com",
    );
}

#[test]
fn the_check_rejects_remote_origins() {
    for source in [
        "http:",
        "https:",
        "https://example.com",
        "http://localhost:1420",
        "ws://localhost:1420",
        "wss:",
        "http://ipc.localhost.example.com",
        "data:",
        "blob:",
    ] {
        for directive in ["script-src", "connect-src", "img-src", "style-src"] {
            assert_one_problem(
                &strict_with(directive, &format!("'self' {source}")),
                &format!("allows {source}"),
            );
        }
    }
}

#[test]
fn the_check_rejects_unsafe_eval_and_unsafe_inline() {
    for source in [
        "'unsafe-eval'",
        "'UNSAFE-EVAL'",
        "'wasm-unsafe-eval'",
        "'unsafe-inline'",
    ] {
        let loose = strict_with("script-src", &format!("'self' {source}"));
        assert_one_problem(&loose, &format!("allows {}", source.to_lowercase()));
    }
}

#[test]
fn the_check_only_allows_ipc_in_connect_src() {
    let loose = strict_with("script-src", "'self' ipc: http://ipc.localhost");
    assert_eq!(problems_in(&loose).len(), 2, "{loose}");
}

#[test]
fn the_check_requires_default_src_and_the_directives_that_must_be_none() {
    assert_one_problem(&strict_without("default-src"), "default-src");
    assert_one_problem(&strict_with("default-src", "'self' 'self'"), "default-src");
    for directive in MUST_BE_NONE {
        assert_one_problem(&strict_without(directive), directive);
        assert_one_problem(&strict_with(directive, "'self'"), directive);
    }
}

#[test]
fn the_check_rejects_a_directive_given_twice() {
    // Browsers enforce the first `img-src`; a checker that kept the last
    // would pass this loose policy.
    let twice = format!("img-src 'self' https://tracker.example.com; {STRICT}; img-src 'self'");
    let found = problems_in(&twice);
    assert!(found.iter().any(|p| p.contains("twice")), "{found:#?}");
    assert_one_problem(&format!("{STRICT}; connect-src 'self'"), "twice");
}
