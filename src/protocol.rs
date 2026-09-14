// The `scifi://` internal protocol.
//
// Two separate handlers are registered — one per webview kind — even
// though they share a scheme name. The content webview's handler simply
// has no route for `/chrome*`, so a web page loaded in a tab can never
// address `scifi://chrome/...` and impersonate SCIFI's own UI. Since
// neither webview shares a WebContext, registering the same scheme name
// independently on each is safe (no duplicate-registration conflict).
//
// Most assets are `&'static str` compiled into the binary. The one
// exception is `/asset/background`, which both handlers also serve: it
// reads whatever the user has picked (see background.rs) from disk at
// request time, since it can change at runtime.

use std::borrow::Cow;

use wry::http::{Request, Response};
use wry::WebViewId;

use crate::background;

const CHROME_HTML: &str = include_str!("../assets/chrome.html");
const CHROME_CSS: &str = include_str!("../assets/chrome.css");
const CHROME_JS: &str = include_str!("../assets/chrome.js");
const HOME_HTML: &str = include_str!("../assets/home.html");
const HOME_CSS: &str = include_str!("../assets/home.css");
const HOME_JS: &str = include_str!("../assets/home.js");
const SETTINGS_HTML: &str = include_str!("../assets/settings.html");
const SETTINGS_CSS: &str = include_str!("../assets/settings.css");
const SETTINGS_JS: &str = include_str!("../assets/settings.js");
const INCOGNITO_HTML: &str = include_str!("../assets/incognito.html");
const INCOGNITO_CSS: &str = include_str!("../assets/incognito.css");

type ProtoResponse = Response<Cow<'static, [u8]>>;

/// No inline script/style anywhere in SCIFI's own pages, so a strict CSP
/// with no 'unsafe-inline' is achievable for the browser's own UI.
/// `frame-ancestors 'none'` backs up the X-Frame-Options header below —
/// SCIFI's own pages are never meant to be embedded in anything.
/// `img-src 'self'` covers the background-image asset without opening
/// the door to remote images.
const CSP: &str =
    "default-src 'self'; img-src 'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'";

fn not_found() -> ProtoResponse {
    Response::builder()
        .status(404)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Cow::Borrowed(b"404 not found in SCIFI".as_slice()))
        .unwrap()
}

/// Serves a `&'static str` asset with zero copies: `include_str!` already
/// gives us a `'static` byte slice, so `Cow::Borrowed` avoids the
/// per-request heap allocation a `.to_vec()` would cost.
fn asset(content_type: &str, body: &'static str) -> ProtoResponse {
    Response::builder()
        .header("Content-Type", content_type)
        .header("Content-Security-Policy", CSP)
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        .header("Referrer-Policy", "no-referrer")
        .body(Cow::Borrowed(body.as_bytes()))
        .unwrap()
}

/// Serves the user's chosen background image straight off disk, if one
/// is set. `Cache-Control: no-store` so a background change is never
/// masked by a cached copy of the old (or a cached 404 of no) image —
/// the visible update on an already-open page still goes through
/// `scifiSetBackground` (see chrome.js/home.js/settings.js), this header
/// just guarantees a *fresh* page load never shows stale art.
fn background_response() -> ProtoResponse {
    match (background::read_current(), background::current_content_type()) {
        (Some(bytes), Some(content_type)) => Response::builder()
            .header("Content-Type", content_type)
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(bytes))
            .unwrap(),
        _ => Response::builder()
            .status(404)
            .header("Cache-Control", "no-store")
            .body(Cow::Borrowed(b"".as_slice()))
            .unwrap(),
    }
}

/// Serves SCIFI's own chrome UI (tab strip + toolbar).
pub fn chrome_protocol_handler() -> impl Fn(WebViewId, Request<Vec<u8>>) -> ProtoResponse {
    move |_id, request| match request.uri().path() {
        "/" | "/index.html" => asset("text/html; charset=utf-8", CHROME_HTML),
        "/chrome.css" => asset("text/css; charset=utf-8", CHROME_CSS),
        "/chrome.js" => asset("application/javascript; charset=utf-8", CHROME_JS),
        "/asset/background" => background_response(),
        _ => not_found(),
    }
}

/// Serves internal pages that can legitimately appear inside a content
/// tab: the new-tab/home page, the Settings page, and the incognito
/// landing page. Deliberately does not serve the chrome routes above —
/// see the module doc comment.
///
/// Routed by *host*, not just path: `scifi://home/`, `scifi://settings/`
/// and `scifi://incognito/` are three different hosts that all happen to
/// request path `/` for their HTML — path-only matching can't tell them
/// apart (and would silently serve Home's markup for all three, since
/// it's listed first). Each page's own CSS/JS is requested as a relative
/// reference, which keeps the *same* host as the page that linked it —
/// e.g. incognito.html's `<script src="home.js">` resolves to
/// `scifi://incognito/home.js`, not `scifi://home/home.js`, so that path
/// is matched under the `incognito` host below, reusing HOME_JS's
/// content rather than duplicating that one small file.
pub fn content_protocol_handler() -> impl Fn(WebViewId, Request<Vec<u8>>) -> ProtoResponse {
    move |_id, request| {
        let uri = request.uri();
        let path = uri.path();

        if path == "/asset/background" {
            return background_response();
        }

        match (uri.host().unwrap_or(""), path) {
            ("home", "/" | "") => asset("text/html; charset=utf-8", HOME_HTML),
            ("home", "/home.css") => asset("text/css; charset=utf-8", HOME_CSS),
            ("home", "/home.js") => asset("application/javascript; charset=utf-8", HOME_JS),

            ("settings", "/" | "") => asset("text/html; charset=utf-8", SETTINGS_HTML),
            ("settings", "/settings.css") => asset("text/css; charset=utf-8", SETTINGS_CSS),
            ("settings", "/settings.js") => asset("application/javascript; charset=utf-8", SETTINGS_JS),

            ("incognito", "/" | "") => asset("text/html; charset=utf-8", INCOGNITO_HTML),
            ("incognito", "/incognito.css") => asset("text/css; charset=utf-8", INCOGNITO_CSS),
            ("incognito", "/home.js") => asset("application/javascript; charset=utf-8", HOME_JS),

            _ => not_found(),
        }
    }
}
