// SCIFI — the native browser for MITOS.
//
// Architecture: one native window hosts exactly two live webview kinds,
// both children of a single gtk::Fixed container so SCIFI works under
// both X11 and Wayland (WebViewBuilder::build_as_child is X11-only on
// Linux; build_gtk with a Fixed container supports both):
//
//   - one "chrome" webview (tab strip + toolbar), always alive, serving
//     the browser's own UI from the internal `scifi://chrome/` route.
//   - a small pool of "content" webviews, one per warm tab, capped at
//     state::MAX_LIVE_TABS. Switching to a cold tab rebuilds its webview
//     from its stored URL; switching away from a tab beyond the cap
//     drops its webview entirely. This bounds peak memory to a fixed
//     number of live WebKitGTK page contexts regardless of how many tabs
//     are open, rather than growing with tab count.
//
// The chrome webview never touches the network — its HTML/CSS/JS are
// compiled into the binary (include_str!) and served only over the
// internal `scifi://` scheme, so there's no attacker-controlled content
// in the trust boundary between the UI and Rust. All UI -> Rust
// communication is a single, strictly-typed JSON command channel (see
// ipc.rs) rather than arbitrary script evaluation.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use gtk::prelude::*;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    platform::unix::WindowExtUnix,
    window::WindowBuilder,
};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebViewBuilder, WebViewBuilderExtUnix};

mod background;
mod history;
mod ipc;
mod protocol;
mod security;
mod state;

use state::AppState;

fn main() -> wry::Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("SCIFI")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .build(&event_loop)
        .expect("failed to create the SCIFI window");
    let window = Rc::new(window);

    // A gtk::Fixed lets us position multiple child webviews with pixel
    // bounds inside the same native window (the chrome strip on top, the
    // active tab's content webview below it).
    let fixed = gtk::Fixed::new();
    let vbox = window
        .default_vbox()
        .expect("tao did not provide a gtk vbox on this window (are we really on Linux/GTK?)");
    vbox.pack_start(&fixed, true, true, 0);
    fixed.show_all();

    let state = Rc::new(RefCell::new(AppState {
        window: window.clone(),
        fixed: fixed.clone(),
        chrome: None,
        tabs: Vec::new(),
        live: HashMap::new(),
        lru: VecDeque::new(),
        active: 0,
        next_id: 1,
        content_size: (1280, 800u32.saturating_sub(state::TOOLBAR_HEIGHT)),
        notice: None,
        tracking_protection: true,
        blocked_count: 0,
        incognito_session_log: Vec::new(),
        devtools_enabled: false,
    }));

    let chrome_state = state.clone();
    let chrome_view = WebViewBuilder::new()
        .with_bounds(Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: LogicalSize::new(1280u32, state::TOOLBAR_HEIGHT).into(),
        })
        .with_url("scifi://chrome/")
        .with_devtools(false)
        .with_custom_protocol("scifi".into(), protocol::chrome_protocol_handler())
        .with_ipc_handler(move |request| {
            ipc::dispatch(&chrome_state, request.body());
        })
        .build_gtk(&fixed)
        .expect("failed to create the SCIFI chrome webview");

    state.borrow_mut().chrome = Some(chrome_view);

    // First tab: SCIFI's own new-tab page.
    state::open_tab(&state, "scifi://home/", false);

    let resize_state = state.clone();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let scale = resize_state.borrow().window.scale_factor();
                let logical = size.to_logical::<u32>(scale);
                state::layout(&resize_state, logical.width, logical.height);
            }
            _ => {}
        }
    });
}
