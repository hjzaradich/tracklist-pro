//! Keeps the webview on the app's own pages.
//!
//! The CSP stops the page from fetching anything remote, but it doesn't
//! cover a top-level navigation (a link, `location.href = …`) or
//! `window.open`, and a remote page loaded into the window would talk to
//! the network outside the gate. So the window refuses to navigate
//! anywhere but the app's own origin, and never opens a new window.

use tauri::webview::NewWindowResponse;
use tauri::{App, Manager, Runtime, Url, WebviewWindowBuilder};

/// Whether `url` is one of the app's own pages: `tauri://localhost`, or
/// `http(s)://tauri.localhost` as WebView2 spells it on Windows. In dev
/// builds, also the dev server (`devUrl` in tauri.conf.json), passed as
/// `dev_url`.
pub fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    let same_origin = |a: &Url, b: &Url| {
        a.scheme() == b.scheme()
            && a.host_str() == b.host_str()
            && a.port_or_known_default() == b.port_or_known_default()
    };
    let bare = url.username().is_empty() && url.password().is_none() && url.port().is_none();
    match url.scheme() {
        "tauri" => bare && url.host_str() == Some("localhost"),
        "http" | "https" if url.host_str() == Some("tauri.localhost") => bare,
        _ => dev_url.is_some_and(|dev| same_origin(url, dev)),
    }
}

/// The dev server's URL in a dev build, `None` in a release build.
pub fn dev_url<R: Runtime, M: Manager<R>>(manager: &M) -> Option<Url> {
    if tauri::is_dev() {
        manager.config().build.dev_url.clone()
    } else {
        None
    }
}

/// `builder`, refusing to navigate off the app's own pages or to open new
/// windows. Every window the app makes goes through this.
pub fn guard<'a, R: Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
    dev_url: Option<Url>,
) -> WebviewWindowBuilder<'a, R, M> {
    builder
        .on_navigation(move |url| is_app_url(url, dev_url.as_ref()))
        .on_new_window(|_, _| NewWindowResponse::Deny)
}

/// Opens the windows listed in tauri.conf.json, each through [`guard`].
/// The config marks them `"create": false`, so Tauri never opens one
/// unguarded.
pub fn open_windows<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    open_windows_with(app, |window| window)
}

/// [`open_windows`], with `customize` applied to each window's builder
/// before the guard is: the development preview's title and browser
/// profile (debug builds only, from `lib.rs`). The guard is added after,
/// so `customize` can't open a window without it.
pub fn open_windows_with<R: Runtime>(
    app: &App<R>,
    customize: impl Fn(WebviewWindowBuilder<'_, R, App<R>>) -> WebviewWindowBuilder<'_, R, App<R>>,
) -> tauri::Result<()> {
    let dev_url = dev_url(app);
    for window in &app.config().app.windows {
        let builder = customize(WebviewWindowBuilder::from_config(app, window)?);
        guard(builder, dev_url.clone()).build()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_window_in_the_config_is_opened_unguarded() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        assert!(!windows.is_empty());
        for window in windows {
            assert_eq!(window["create"], false, "{window}");
        }
    }

    fn url(s: &str) -> Url {
        s.parse().unwrap()
    }

    #[test]
    fn the_apps_own_pages_are_allowed() {
        for ok in [
            "tauri://localhost",
            "tauri://localhost/index.html#/library",
            "http://tauri.localhost/",
            "http://tauri.localhost/crates?x=1",
            "https://tauri.localhost/",
        ] {
            assert!(is_app_url(&url(ok), None), "{ok}");
        }
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "https://example.com/",
            "http://musicbrainz.org/",
            "http://localhost:1420/", // the dev server, in a release build
            "http://tauri.localhost.example.com/",
            "http://evil.tauri.localhost/",
            "http://tauri.localhost:8080/",
            "http://user@tauri.localhost/",
            "tauri://evil.com/",
            "file:///C:/Windows/win.ini",
            "about:blank",
            "javascript:alert(1)",
            "data:text/html,<p>hi</p>",
            "ftp://tauri.localhost/",
        ] {
            assert!(!is_app_url(&url(bad), None), "{bad}");
        }
    }

    #[test]
    fn a_dev_build_also_allows_only_the_dev_servers_origin() {
        let dev = url("http://localhost:1420");
        assert!(is_app_url(&url("http://localhost:1420/"), Some(&dev)));
        assert!(is_app_url(
            &url("http://localhost:1420/src/main.tsx"),
            Some(&dev)
        ));
        for bad in [
            "http://localhost:1421/",
            "https://localhost:1420/",
            "http://127.0.0.1:1420/",
            "https://example.com/",
        ] {
            assert!(!is_app_url(&url(bad), Some(&dev)), "{bad}");
        }
    }
}
