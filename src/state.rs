// Tab and webview-pool state for SCIFI.
//
// Memory-efficiency design: only up to MAX_LIVE_TABS content webviews are
// ever alive at once, tracked in `live` with recency order in `lru`.
// Every open tab (warm or not) has an entry in `tabs` with its url/title;
// only the warm ones additionally have a real wry::WebView. Switching to
// a cold tab rebuilds a fresh webview from its stored url (a small,
// deliberate trade-off: memory stays bounded no matter how many tabs are
// open, at the cost of a reload — and a lost scroll/form position — when
// returning to a tab that fell out of the warm set). Incognito tabs are
// no different here: reopening one beyond the warm-tab cap gets a brand
// new ephemeral session, which if anything is more private, not less.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use serde::Serialize;
use tao::window::Window;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebView, WebViewBuilder, WebViewBuilderExtUnix};

use crate::background;
use crate::history;
use crate::ipc;
use crate::protocol;
use crate::security::{self, BlockReason, Verdict};

/// Combined height (px) of the chrome webview: outer padding + the
/// floating tab-strip row + the gap + the floating toolbar row. Must
/// match the layout assets/chrome.css actually renders (8 + 32 + 6 + 44
/// + 6 = 96) — if you change the CSS spacing, update this too.
pub const TOOLBAR_HEIGHT: u32 = 96;

/// Maximum number of content webviews kept warm at once. Tabs beyond this
/// count are evicted in least-recently-used order: their WebView is
/// dropped (freeing its WebKitGTK page/process resources), but the tab's
/// url/title stay in `tabs`, so revisiting it just reloads the page.
pub const MAX_LIVE_TABS: usize = 6;

/// A custom, SCIFI-flavored user agent so sites can tell they're being
/// visited by SCIFI rather than a bare WebKitGTK build.
const USER_AGENT: &str = "SCIFI/0.1 (MITOS; Linux) WebKitGTK";

pub struct TabMeta {
    pub id: u32,
    pub url: String,
    pub title: String,
    pub incognito: bool,
}

pub struct AppState {
    pub window: Rc<Window>,
    pub fixed: gtk::Fixed,
    pub chrome: Option<WebView>,
    pub tabs: Vec<TabMeta>,
    pub live: HashMap<u32, WebView>,
    pub lru: VecDeque<u32>,
    pub active: u32,
    pub next_id: u32,
    pub content_size: (u32, u32),
    /// A short-lived, user-visible notice (e.g. "blocked: ..."), shown in
    /// the toolbar and cleared on the next successful navigation.
    pub notice: Option<String>,
    /// Whether the tracker/ad host blocklist is enforced. Toggleable from
    /// the Settings page (scifi://settings/); the scheme allowlist is a
    /// separate, always-on security boundary this does not affect.
    pub tracking_protection: bool,
    /// Count of navigations blocked specifically for matching the tracker
    /// blocklist, this session. Shown on the Settings page.
    pub blocked_count: u32,
    /// Visits from incognito tabs this session — RAM only, never
    /// touches disk. Cleared automatically once the last incognito tab
    /// closes (see close_tab), and manually clearable from Settings.
    pub incognito_session_log: Vec<history::HistoryEntry>,
    /// Whether new content webviews are built with devtools enabled
    /// (right-click → Inspect Element becomes available). Off by
    /// default — see the README for why this is opt-in rather than
    /// always-on. Takes effect for webviews built *after* the toggle
    /// flips; an already-open tab keeps whatever it was built with
    /// until it next gets rebuilt (a fresh tab, or a cold tab reopened).
    pub devtools_enabled: bool,
}

#[derive(Serialize)]
struct ChromeTab {
    id: u32,
    title: String,
    url: String,
    is_active: bool,
    is_incognito: bool,
}

#[derive(Serialize)]
struct ChromeState {
    tabs: Vec<ChromeTab>,
    active_url: String,
    can_go_back: bool,
    can_go_forward: bool,
    is_secure: bool,
    is_incognito: bool,
    notice: Option<String>,
}

#[derive(Serialize)]
struct SettingsState {
    tracking_protection: bool,
    blocked_count: u32,
    blocked_hosts: Vec<&'static str>,
    background_set: bool,
    devtools_enabled: bool,
    history: Vec<history::HistoryEntry>,
    private_session: Vec<history::HistoryEntry>,
}

fn content_rect(width: u32, height: u32) -> Rect {
    Rect {
        position: LogicalPosition::new(0, TOOLBAR_HEIGHT).into(),
        size: LogicalSize::new(width, height).into(),
    }
}

/// Builds a fresh content webview for `url`, wired up with SCIFI's
/// navigation policy, internal protocol, and state-sync handlers.
///
/// `incognito` maps directly to wry's own `with_incognito`, which on
/// WebKitGTK creates the view with an ephemeral (non-persistent)
/// browsing session — cookies, cache, and local storage for it are
/// never written to disk and are gone once the webview is dropped.
pub fn build_content_webview(state: &Rc<RefCell<AppState>>, url: &str, incognito: bool) -> WebView {
    let st_load = state.clone();
    let st_title = state.clone();
    let st_new = state.clone();
    let st_nav = state.clone();
    let st_ipc = state.clone();

    let (fixed, devtools_enabled) = {
        let s = state.borrow();
        (s.fixed.clone(), s.devtools_enabled)
    };

    let mut builder = WebViewBuilder::new()
        .with_url(url)
        .with_user_agent(USER_AGENT)
        .with_devtools(devtools_enabled)
        .with_custom_protocol("scifi".into(), protocol::content_protocol_handler())
        .with_navigation_handler(move |target| {
            let tracking_protection = st_nav.borrow().tracking_protection;
            match security::classify(&target, tracking_protection) {
                Verdict::Allow => true,
                Verdict::Block(reason) => {
                    block_and_notify(&st_nav, &target, reason);
                    false
                }
            }
        })
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, wry::PageLoadEvent::Finished) {
                let title = {
                    let active = st_load.borrow().active;
                    let mut s = st_load.borrow_mut();
                    let mut title = String::new();
                    if let Some(tab) = s.tabs.iter_mut().find(|t| t.id == active) {
                        tab.url = url.clone();
                        title = tab.title.clone();
                    }
                    s.notice = None;
                    title
                };
                // Incognito visits never touch disk — they go in the
                // in-memory session log instead, which close_tab clears
                // once the last incognito tab closes.
                if incognito {
                    let mut s = st_load.borrow_mut();
                    s.incognito_session_log.push(history::HistoryEntry {
                        url: url.clone(),
                        title,
                        visited_at: history::now_unix_secs(),
                    });
                } else {
                    history::record_visit(&url, &title);
                }
                sync_chrome(&st_load);
                sync_settings_pages(&st_load);
            }
        })
        .with_document_title_changed_handler(move |title| {
            {
                let active = st_title.borrow().active;
                let mut s = st_title.borrow_mut();
                if let Some(tab) = s.tabs.iter_mut().find(|t| t.id == active) {
                    tab.title = title;
                }
            }
            sync_chrome(&st_title);
        })
        .with_new_window_req_handler(move |req_url, _features| {
            // Route window.open()/target=_blank into SCIFI's own tab
            // model instead of letting an ungoverned native window open
            // — and run it through the same navigation policy a regular
            // link would get, so a page can't use a popup request to
            // reach a scheme/host normal navigation would have blocked
            // (this also stops popup-spam from filling the tab strip). A
            // popup opened from an incognito tab stays incognito.
            let tracking_protection = st_new.borrow().tracking_protection;
            match security::classify(&req_url, tracking_protection) {
                Verdict::Allow => {
                    open_tab(&st_new, &req_url, incognito);
                }
                Verdict::Block(reason) => {
                    block_and_notify(&st_new, &req_url, reason);
                }
            }
            wry::NewWindowResponse::Deny
        })
        // This is NOT a general content-webview IPC channel — arbitrary
        // pages can technically call window.ipc.postMessage (wry injects
        // it into every webview that has a handler registered at all),
        // but the handler below only ever acts on a message if the
        // *document that sent it* is SCIFI's own Settings page, checked
        // via request.uri() rather than anything the page's own JS
        // could claim. (On Linux, if the message came from an iframe,
        // wry reports the main-frame URL instead of the iframe's — which
        // only makes this check stricter, since SCIFI never loads its
        // own pages inside a frame, and a page trying to iframe
        // scifi://settings to spoof this gets its own top-level URL
        // reported instead, not scifi://settings's. That page also can't
        // be framed at all per its CSP — see protocol.rs.) Worst case if
        // this check somehow didn't hold: a page could flip the tracking
        // protection toggle or the saved background — no access to
        // history, credentials, or anything off this device.
        .with_ipc_handler(move |request| {
            let uri = request.uri();
            let is_settings_page =
                uri.scheme_str() == Some("scifi") && uri.host() == Some("settings");
            if is_settings_page {
                ipc::dispatch_settings(&st_ipc, request.body());
            }
        });

    if incognito {
        builder = builder.with_incognito(true);
    }

    builder
        .build_gtk(&fixed)
        .expect("failed to create a SCIFI content webview")
}

fn block_and_notify(state: &Rc<RefCell<AppState>>, target: &str, reason: BlockReason) {
    {
        let mut s = state.borrow_mut();
        s.notice = Some(format!("Blocked: {target} ({reason})"));
        if reason.is_tracker() {
            s.blocked_count = s.blocked_count.saturating_add(1);
        }
    }
    sync_chrome(state);
    if reason.is_tracker() {
        sync_settings_pages(state);
    }
}

/// Makes `id` the active tab: building its webview if it's currently
/// cold, hiding the previously active one, and applying LRU eviction if
/// the warm set has grown past MAX_LIVE_TABS.
pub fn activate_tab(state: &Rc<RefCell<AppState>>, id: u32) {
    let prev_active = state.borrow().active;
    if prev_active != id {
        if let Some(prev_view) = state.borrow().live.get(&prev_active) {
            let _ = prev_view.set_visible(false);
        }
    }

    let needs_build = !state.borrow().live.contains_key(&id);
    if needs_build {
        let (url, incognito) = {
            let s = state.borrow();
            s.tabs
                .iter()
                .find(|t| t.id == id)
                .map(|t| (t.url.clone(), t.incognito))
                .unwrap_or_else(|| ("scifi://home/".to_string(), false))
        };
        let view = build_content_webview(state, &url, incognito);
        state.borrow_mut().live.insert(id, view);
    }

    let (w, h) = state.borrow().content_size;
    if let Some(view) = state.borrow().live.get(&id) {
        let _ = view.set_bounds(content_rect(w, h));
        let _ = view.set_visible(true);
    }

    {
        let mut s = state.borrow_mut();
        s.active = id;
        s.lru.retain(|&t| t != id);
        s.lru.push_front(id);
        while s.lru.len() > MAX_LIVE_TABS {
            match s.lru.pop_back() {
                Some(evict_id) => {
                    s.live.remove(&evict_id);
                }
                None => break,
            }
        }
    }

    sync_chrome(state);
}

/// Opens a brand-new tab at `url` and makes it active. Returns the new
/// tab's id.
pub fn open_tab(state: &Rc<RefCell<AppState>>, url: &str, incognito: bool) -> u32 {
    let id = {
        let mut s = state.borrow_mut();
        let id = s.next_id;
        s.next_id += 1;
        s.tabs.push(TabMeta {
            id,
            url: url.to_string(),
            title: "New Tab".to_string(),
            incognito,
        });
        s.notice = None;
        id
    };
    activate_tab(state, id);
    id
}

/// Closes tab `id`. If it was the active tab, activates the last
/// remaining tab, or opens a fresh home tab if none are left. If that
/// was the last open incognito tab, the in-memory incognito session log
/// is cleared too — closing your last private tab ends that session.
pub fn close_tab(state: &Rc<RefCell<AppState>>, id: u32) {
    let was_active = state.borrow().active == id;
    {
        let mut s = state.borrow_mut();
        s.tabs.retain(|t| t.id != id);
        s.live.remove(&id);
        s.lru.retain(|&t| t != id);
        if !s.tabs.iter().any(|t| t.incognito) {
            s.incognito_session_log.clear();
        }
    }
    if !was_active {
        sync_chrome(state);
        sync_settings_pages(state);
        return;
    }
    let next = state.borrow().tabs.last().map(|t| t.id);
    match next {
        Some(next_id) => activate_tab(state, next_id),
        None => {
            open_tab(state, "scifi://home/", false);
        }
    }
}

/// Resolves and navigates the active tab to whatever the user typed in
/// the address bar (or a `navigate` IPC command carries).
pub fn navigate_active(state: &Rc<RefCell<AppState>>, raw_input: &str) {
    let tracking_protection = state.borrow().tracking_protection;
    let resolution = security::resolve_address_bar_input(raw_input, tracking_protection);

    let target = match resolution {
        security::AddressResolution::Url(url) => url,
        security::AddressResolution::Empty => return,
        security::AddressResolution::NotAnAddress => {
            {
                let mut s = state.borrow_mut();
                s.notice = Some(format!(
                    "Can't open \"{raw_input}\" \u{2014} try a full address like https://example.com"
                ));
            }
            sync_chrome(state);
            return;
        }
        security::AddressResolution::Blocked(reason) => {
            block_and_notify(state, raw_input, reason);
            return;
        }
    };

    let active = state.borrow().active;
    if let Some(view) = state.borrow().live.get(&active) {
        let _ = view.load_url(&target);
    }
    {
        let mut s = state.borrow_mut();
        if let Some(tab) = s.tabs.iter_mut().find(|t| t.id == active) {
            tab.url = target.clone();
        }
        s.notice = None;
    }
    sync_chrome(state);
}

/// Repositions the chrome webview and the active content webview after a
/// window resize. Warm-but-hidden webviews are left alone until they're
/// shown again (activate_tab sets their bounds at that point) — no point
/// laying out something nobody can see.
pub fn layout(state: &Rc<RefCell<AppState>>, width: u32, height: u32) {
    let content_h = height.saturating_sub(TOOLBAR_HEIGHT);
    {
        let s = state.borrow();
        if let Some(chrome) = &s.chrome {
            let _ = chrome.set_bounds(Rect {
                position: LogicalPosition::new(0, 0).into(),
                size: LogicalSize::new(width, TOOLBAR_HEIGHT).into(),
            });
        }
        if let Some(view) = s.live.get(&s.active) {
            let _ = view.set_bounds(content_rect(width, content_h));
        }
    }
    state.borrow_mut().content_size = (width, content_h);
}

/// Pushes the current tab list / navigation state / notice to the chrome
/// UI, and updates the OS window title to match the active tab.
pub fn sync_chrome(state: &Rc<RefCell<AppState>>) {
    let s = state.borrow();
    let active_view = s.live.get(&s.active);
    let can_go_back = active_view
        .and_then(|v| v.can_go_back().ok())
        .unwrap_or(false);
    let can_go_forward = active_view
        .and_then(|v| v.can_go_forward().ok())
        .unwrap_or(false);
    let active_tab = s.tabs.iter().find(|t| t.id == s.active);
    let active_url = active_tab.map(|t| t.url.clone()).unwrap_or_default();
    let active_title = active_tab
        .map(|t| t.title.clone())
        .unwrap_or_else(|| "SCIFI".to_string());
    let is_secure = active_url.starts_with("https://");
    let is_incognito = active_tab.map(|t| t.incognito).unwrap_or(false);

    let payload = ChromeState {
        tabs: s
            .tabs
            .iter()
            .map(|t| ChromeTab {
                id: t.id,
                title: t.title.clone(),
                url: t.url.clone(),
                is_active: t.id == s.active,
                is_incognito: t.incognito,
            })
            .collect(),
        active_url,
        can_go_back,
        can_go_forward,
        is_secure,
        is_incognito,
        notice: s.notice.clone(),
    };

    if let (Some(chrome), Ok(json)) = (&s.chrome, serde_json::to_string(&payload)) {
        let _ = chrome.evaluate_script(&format!("window.scifiRender({json})"));
    }
    s.window.set_title(&format!(
        "{}{} \u{2014} SCIFI",
        if is_incognito { "\u{1F576} " } else { "" },
        active_title
    ));
}

/// Pushes current privacy/appearance state to every live webview whose
/// tab is on the Settings page — there could be more than one open.
pub fn sync_settings_pages(state: &Rc<RefCell<AppState>>) {
    let s = state.borrow();
    let payload = SettingsState {
        tracking_protection: s.tracking_protection,
        blocked_count: s.blocked_count,
        blocked_hosts: security::BLOCKED_HOSTS.to_vec(),
        background_set: background::current_path().is_some(),
        devtools_enabled: s.devtools_enabled,
        history: history::load_recent(500),
        private_session: s.incognito_session_log.clone(),
    };
    let json = match serde_json::to_string(&payload) {
        Ok(j) => j,
        Err(_) => return,
    };
    for (id, view) in s.live.iter() {
        let is_settings_tab = s
            .tabs
            .iter()
            .any(|t| t.id == *id && t.url.starts_with("scifi://settings"));
        if is_settings_tab {
            let _ = view.evaluate_script(&format!("window.scifiSettingsRender({json})"));
        }
    }
}

/// Tells every live webview (chrome and every open content tab) to
/// re-fetch the background image with a fresh cache-busting token, so a
/// background change is visible immediately on any already-open page —
/// not just the next one that loads. Harmless no-op on pages that don't
/// define `window.scifiSetBackground` (i.e. any regular website).
pub fn refresh_backgrounds(state: &Rc<RefCell<AppState>>) {
    let s = state.borrow();
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let script = format!("window.scifiSetBackground && window.scifiSetBackground({token})");

    if let Some(chrome) = &s.chrome {
        let _ = chrome.evaluate_script(&script);
    }
    for view in s.live.values() {
        let _ = view.evaluate_script(&script);
    }
}
