// IPC command channels from webview JS to Rust.
//
// There are two, with different trust levels:
//
// - `dispatch`: the chrome UI's channel. The chrome webview only ever
//   shows SCIFI's own compiled-in HTML/JS, so this channel trusts any
//   well-formed command it receives.
// - `dispatch_settings`: a narrow channel for the Settings page
//   (scifi://settings/), which runs inside a *content* webview — the
//   same kind of webview that shows arbitrary websites. The caller
//   (state::build_content_webview) only forwards a message here after
//   confirming, via the message's own request URI, that it actually came
//   from scifi://settings and not from whatever a content webview might
//   currently be showing.
//
// Both parse straight into a strictly-typed enum with serde; anything
// malformed or unrecognized is silently ignored rather than evaluated or
// echoed back.

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;
use tao::platform::unix::WindowExtUnix;

use crate::background;
use crate::state::{self, AppState};

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum ChromeCommand {
    Navigate { url: String },
    NewTab { url: Option<String> },
    NewIncognitoTab,
    CloseTab { id: u32 },
    SwitchTab { id: u32 },
    GoBack,
    GoForward,
    Reload,
}

/// Handles one message posted from the chrome UI.
pub fn dispatch(state: &Rc<RefCell<AppState>>, raw: &str) {
    let cmd: ChromeCommand = match serde_json::from_str(raw) {
        Ok(c) => c,
        Err(_) => return,
    };

    match cmd {
        ChromeCommand::Navigate { url } => state::navigate_active(state, &url),
        ChromeCommand::NewTab { url } => {
            let target = url.unwrap_or_else(|| "scifi://home/".to_string());
            state::open_tab(state, &target, false);
        }
        ChromeCommand::NewIncognitoTab => {
            state::open_tab(state, "scifi://incognito/", true);
        }
        ChromeCommand::CloseTab { id } => state::close_tab(state, id),
        ChromeCommand::SwitchTab { id } => state::activate_tab(state, id),
        ChromeCommand::GoBack => with_active_view(state, |v| {
            let _ = v.go_back();
        }),
        ChromeCommand::GoForward => with_active_view(state, |v| {
            let _ = v.go_forward();
        }),
        ChromeCommand::Reload => with_active_view(state, |v| {
            let _ = v.reload();
        }),
    }
}

fn with_active_view(state: &Rc<RefCell<AppState>>, f: impl FnOnce(&wry::WebView)) {
    let active = state.borrow().active;
    if let Some(view) = state.borrow().live.get(&active) {
        f(view);
    }
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum SettingsCommand {
    SetTrackingProtection { enabled: bool },
    PickBackground,
    ResetBackground,
    ClearHistory,
    ClearPrivateSession,
    SetDevtools { enabled: bool },
}

/// Handles one message posted from the Settings page. Only ever called
/// after the caller has verified the message's request URI is actually
/// scifi://settings — see the module doc comment above and the
/// with_ipc_handler comment in state.rs.
pub fn dispatch_settings(state: &Rc<RefCell<AppState>>, raw: &str) {
    let cmd: SettingsCommand = match serde_json::from_str(raw) {
        Ok(c) => c,
        Err(_) => return,
    };

    match cmd {
        SettingsCommand::SetTrackingProtection { enabled } => {
            {
                let mut s = state.borrow_mut();
                s.tracking_protection = enabled;
            }
            state::sync_settings_pages(state);
        }
        SettingsCommand::PickBackground => {
            // Clone the gtk handle out and drop the state borrow *before*
            // the blocking dialog call below — dialog.run() enters a
            // nested main loop, during which other async callbacks (e.g.
            // another tab finishing a page load) can still fire, and any
            // of them needing state.borrow_mut() while we held an
            // outstanding borrow() here would panic.
            let parent = state.borrow().window.gtk_window().clone();
            let changed = background::pick_and_set(&parent);
            if changed {
                state::sync_settings_pages(state);
                state::refresh_backgrounds(state);
            }
        }
        SettingsCommand::ResetBackground => {
            background::clear();
            state::sync_settings_pages(state);
            state::refresh_backgrounds(state);
        }
        SettingsCommand::ClearHistory => {
            crate::history::clear();
            state::sync_settings_pages(state);
        }
        SettingsCommand::ClearPrivateSession => {
            {
                let mut s = state.borrow_mut();
                s.incognito_session_log.clear();
            }
            state::sync_settings_pages(state);
        }
        SettingsCommand::SetDevtools { enabled } => {
            {
                let mut s = state.borrow_mut();
                s.devtools_enabled = enabled;
            }
            state::sync_settings_pages(state);
        }
    }
}
