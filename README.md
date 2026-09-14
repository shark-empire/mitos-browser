# SCIFI

SCIFI is the native browser for **MITOS** — a Rust webview-shell browser
built on `wry` (WebKitGTK on Linux), tuned for speed, security, and
memory efficiency on real desktop/laptop hardware.

SCIFI is a *webview-shell* browser, not a from-scratch rendering engine:
`wry` wraps WebKitGTK for the actual page rendering/JS/networking, and
this crate provides the native chrome (tab strip, toolbar, tab
management, navigation policy) around it.

## Architecture

One native window hosts two kinds of webview, both children of a single
`gtk::Fixed` container (this — rather than `build_as_child` — is what
gets SCIFI working under both X11 and Wayland):

- **Chrome webview** (always alive): the tab strip + toolbar, served
  entirely from the internal `scifi://chrome/` route. Its HTML/CSS/JS are
  compiled into the binary (`include_str!`) — never fetched from the
  network — so there's no attacker-controlled content on the UI side of
  the trust boundary. It talks to Rust over a single strictly-typed JSON
  IPC command channel (`src/ipc.rs`); malformed or unrecognized messages
  are dropped, never evaluated.
- **Content webviews** (a small warm pool): one per tab that's actually
  been visited recently, capped at `state::MAX_LIVE_TABS` (default 6).
  Switching to a "cold" tab (beyond the cap) rebuilds its webview from
  its stored URL; switching away from one evicts the least-recently-used
  one once the cap is exceeded. This bounds peak memory to a fixed number
  of live WebKitGTK page contexts regardless of how many tabs are open —
  the trade-off is that a reopened cold tab reloads from scratch (no
  restored scroll position/form state). Incognito tabs live in the same
  pool under the same rules (see Privacy & incognito below).

```
src/
  main.rs        window + event loop, wires everything together
  state.rs       Tab, AppState, the warm-webview pool + LRU eviction
  protocol.rs    the scifi:// scheme (serves every asset set below)
  security.rs    scheme allowlist + toggleable host blocklist
  ipc.rs         chrome-UI and Settings-page command enums + dispatch
  background.rs  native file picker + on-disk custom background
  history.rs     persisted history (background writer) for normal tabs
assets/
  chrome.html/css/js       the tab strip + toolbar UI
  home.html/css/js         the new-tab page
  settings.html/css/js     Privacy, Appearance, History, Advanced
  incognito.html/css       the incognito landing page
```

## Security

- **Scheme allowlist**: only `https`, `http`, and the internal `scifi:`
  are navigable — `file:`, `javascript:`, and `data:` are refused, both
  for typed address-bar input and for in-page navigation (links,
  redirects, JS-triggered navigation), via `with_navigation_handler`.
- **Host blocklist**: a short illustrative list of known tracker/ad
  domains in `security.rs` — swap in a maintained list for real coverage.
- **Chrome/content isolation**: the content webview's `scifi://` handler
  has no route for `/chrome*`, so a malicious page can never navigate
  itself to `scifi://chrome/...` and spoof SCIFI's own UI.
- **No devtools in release builds**: the `devtools` cargo feature is
  deliberately *not* enabled, so `with_devtools` only has effect in debug
  builds (this is `wry`'s own default gate) — release builds physically
  can't expose a devtools attack surface to a compromised page.
- **Strict headers** on every SCIFI-served page: a CSP of `default-src
  'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'`
  (achievable with no `'unsafe-inline'` because none of SCIFI's own HTML
  uses inline scripts/styles), plus `X-Frame-Options: DENY`,
  `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`.
- **Popups routed through SCIFI's own tab model, not exempted from
  policy**: `window.open()` / `target=_blank` requests are denied at the
  native-window level; the URL is run through the same scheme/host check
  as a normal navigation before SCIFI will open it as a tab, and a
  disallowed one surfaces the toolbar notice instead — otherwise a page
  could use popups to reach what normal navigation would have blocked,
  or just spam the tab strip.

## Privacy & incognito

- **Incognito tabs are a real separate session, not a label**: new
  incognito tabs (toolbar button, or opened from a popup spawned by
  another incognito tab) are built with wry's `with_incognito(true)`,
  which on WebKitGTK gives the webview its own ephemeral browsing
  session — cookies, cache, and local storage for it are never written
  to disk, and are gone as soon as that tab's webview is dropped (either
  on close, or on LRU eviction — revisiting an evicted incognito tab
  starts a fresh session, which if anything is *more* private, not
  less). The toolbar, tab pill, and window title all shift to a violet
  accent while an incognito tab is active, so it's obvious at a glance.
- **Tracking protection is a real toggle, not just a static list**:
  `security::classify` takes a `tracking_protection: bool` read fresh
  from `AppState` on every navigation — the Settings page's switch
  flips that bit directly, no restart needed. The scheme allowlist
  (`file:`/`javascript:`/`data:` refusal) is separate and always on
  regardless of the toggle; only the tracker/ad host blocklist is gated
  by it.
- **The Settings page (`scifi://settings/`) runs inside a content
  webview**, the same kind that shows arbitrary websites — so its IPC
  channel (`ipc::dispatch_settings`) is intentionally narrow. wry
  injects `window.ipc.postMessage` into every webview with a handler at
  all, so a regular website *can* call it, but the handler only acts on
  a message once it's confirmed, via the message's own request URI
  (`request.uri()`, which reflects the sending document — see wry's own
  docs on `with_ipc_handler`), that it actually came from
  `scifi://settings` and not from whatever the tab currently shows.
  Worst case if that check somehow didn't hold: a page could flip the
  tracking-protection toggle or the saved background image — no access
  to history, credentials, or anything else on the device.

## Custom backgrounds

The Settings page's Appearance section lets you pick a local image
(native GTK file picker — `gtk::FileChooserNative`, which uses the
desktop's file portal where available, so it also works correctly in a
sandboxed environment) to show, blurred, behind the tab strip, toolbar,
and every internal SCIFI page. SCIFI never decodes or resizes it — the
original file is copied byte-for-byte into `$XDG_DATA_HOME/scifi/` (or
`~/.local/share/scifi/`) as `background.<original-extension>` and served
back as-is over `scifi://asset/background`, read from disk on request so
picking a new one takes effect without a rebuild or restart. A dark
scrim is layered on top of it in CSS so toolbar text stays legible
regardless of what photo you pick. If no background is set, that image
layer simply fails to load and SCIFI's default gradient shows through
underneath — no Rust ↔ JS "is one set" round trip needed for that part.

## History & incognito session log

- **Normal tabs**: every completed page load (`PageLoadEvent::Finished`)
  on a non-incognito tab is handed off to `history.rs`, which appends
  `{url, title, visited_at}` as one JSON line to
  `$XDG_DATA_HOME/scifi/history.ndjson` on a small background thread —
  never blocking the GTK main thread on disk I/O. No database
  dependency: newline-delimited JSON is crash-safe by construction (a
  torn last line just fails to parse and is skipped) and trivial to
  inspect or wipe by hand. SCIFI's own internal pages
  (`scifi://...`) are never logged.
- **Incognito tabs** never call into `history.rs` at all. Their visits go
  into `AppState::incognito_session_log` instead — RAM only, and cleared
  automatically the moment your last open incognito tab closes (or
  manually, any time, from Settings). This preserves the actual privacy
  guarantee rather than just hiding incognito visits from view while
  still writing them somewhere.
- Both show up on the Settings page (`scifi://settings/`): "Private tabs
  this session" (the ephemeral list above) and "History" (persisted,
  grouped by day, with a client-side search box — filtering doesn't
  round-trip to Rust, it's just a substring match over whatever's
  already loaded). Entries are real `<a href>` elements, so clicking one
  navigates the Settings tab there through the normal navigation path —
  same policy check as any other link, no separate "navigate to history
  entry" IPC command needed.
- `history::load_recent` caps what's sent to the page at 500 entries so
  a long-lived history doesn't mean shipping an ever-growing blob to
  the UI on every sync; raise the cap in `state::sync_settings_pages` if
  you want more.

## Developer tools (right-click → Inspect Element)

WebKit's *native* right-click context menu already includes "Inspect
Element" once devtools are enabled for a webview — there's no custom
context menu to build here. What SCIFI adds is a Settings toggle
(off by default) that controls `with_devtools` for content webviews
built from then on; an already-open tab keeps whatever it was built
with until it's next rebuilt (a fresh tab, or a cold tab reopened).

Worth being direct about: **this partly reverses an earlier decision.**
The original security write-up in this README explained why devtools
were deliberately left *off* in release builds — enabling devtools
mostly matters for something like the well-known "paste this into your
console" social-engineering pattern, since the inspector only reaches
content already inside that webview's own page, nothing in Rust, other
tabs, or the filesystem. Given SCIFI is being built *by* its own
developer, an always-available inspector is a reasonable thing to want.
The compromise here is the default-off toggle rather than turning it on
unconditionally: the `devtools` cargo feature had to be enabled for the
toggle to do anything in release builds at all (debug builds always have
it regardless), so the *capability* now exists in the compiled binary —
the runtime toggle is what keeps it off unless deliberately switched on.
If that trade-off is wrong for a given build, dropping the `devtools`
feature from Cargo.toml is the way to remove the capability entirely
again.

One more thing this doesn't cover: WebKit's context menu is used as-is,
not rebuilt from scratch, so items beyond Inspect Element (View Page
Source, Open Link in New Window, etc.) are whatever WebKit ships by
default rather than individually vetted here. "Open Link in New Window"
in particular is expected to route through the same
`with_new_window_req_handler` this README already documents (context
menu "open in new window" and `window.open()` both fire WebKit's
underlying `create-web-view` signal), so it should already be caught by
SCIFI's tab model and navigation policy rather than bypassing them — but
that's an expectation carried over from how the signal is documented to
work, not something exercised against a real right-click yet.

## Speed & memory

- Release profile: `opt-level = 3`, `lto = true`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true`.
- Chrome UI assets are embedded with zero runtime I/O and served via
  `Cow::Borrowed` (zero-copy — no per-request heap allocation). The one
  exception is the background image, which is necessarily read from
  disk per-request (`Cache-Control: no-store`, so a change is never
  masked by a stale cached copy) since it can change at runtime.
- Hand-written vanilla JS/CSS throughout — no framework/bundler weight,
  instant parse.
- State pushed to the UI only on actual changes (navigation, tab
  open/close/switch), not polled — no idle CPU wakeups.
- The warm-webview pool (above) bounds memory to a fixed number of live
  page contexts rather than growing with tab count.
- Hidden/background webviews aren't just visually hidden
  (`set_visible(false)`) — WebKit's own default suspend policy already
  throttles timers in views that are minimized or hidden, on top of
  whatever SCIFI's eviction does. (`wry`'s
  `with_disable_background_throttling`, which would opt *out* of that,
  is a no-op on Linux — so on this platform the throttling simply stays
  on, which is what we want here.)
- History writes are handed to a dedicated background thread over a
  channel (`history.rs`) rather than written inline in the page-load
  handler — a slow disk shouldn't be able to stall page navigation.

## Look

The chrome UI is a glass HUD: floating translucent panels
(`backdrop-filter: blur()`) over a dark gradient base, a cyan-teal accent
for the secure/active states, hand-drawn inline SVG icons (no icon-font
dependency), and restrained motion (hover/focus transitions, one small
entrance animation for the address-bar notice icon — nothing looping or
decorative). `backdrop-filter` has been supported in WebKitGTK since
2.29.4 (mid-2020), so it should render correctly on any WebKitGTK from
the last several years; on something unusually old it degrades to a
plain translucent panel (no blur) rather than breaking, since the
`rgba()` background is there regardless of blur support.

The chrome webview stayed a fixed height (`state::TOOLBAR_HEIGHT`, now
96px to fit the floating tab-strip + toolbar layout) — if you change
`assets/chrome.css`'s spacing, update that constant to match, or the
content webview underneath will be mispositioned by the difference.

## Build prerequisites

This links against system WebKitGTK/GTK, which `cargo` can't install for
you. On a Debian/Ubuntu-family system:

```
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev
```

(Note the `4.1` suffix, not the older `4.0` — that's the version this
`wry`/`webkit2gtk-rs` pairing links against.) Substitute MITOS's own
package names once `mitos-pkg` can provide these.

```
cargo build --release
```

## Tuning

- `state::MAX_LIVE_TABS` — raise for snappier tab switching at the cost
  of more resident memory; lower for a tighter memory ceiling.
- `security::BLOCKED_HOSTS` / `ALLOWED_SCHEMES` — extend the navigation
  policy.
- `state::USER_AGENT` — SCIFI's UA string sent to sites.
- `history::load_recent`'s cap (500, passed from
  `state::sync_settings_pages`) — how much history the Settings page is
  sent per sync.
- `AppState::devtools_enabled`'s default (`false` in `main.rs`) — flip
  if you'd rather ship with devtools on by default; see "Developer
  tools" above for the trade-off.
## A note on verification

This was written and reasoned through against `wry`'s actual 0.56 /
`tao` 0.36 API (confirmed via docs.rs and the wry changelog — the
version originally pinned in this repo, `wry = "0.40"`, doesn't actually
match the two-argument `with_custom_protocol`/`with_ipc_handler` shapes
that make a multi-webview browser like this work; that shape only landed
at 0.46+, which is why the pin was bumped), but it has **not** been
compiled in this environment (no network access to fetch crates). One
routing bug already turned up from re-reading the code carefully rather
than from a compiler (`content_protocol_handler` was matching only on
`request.uri().path()`, but `scifi://home/`, `scifi://settings/`, and
`scifi://incognito/` are three different *hosts* that each request path
`/` — so it would have silently served Home's markup for all three,
since that arm was listed first; fixed by matching on `(host, path)`
together). That's the kind of thing `cargo build` won't catch either
(it's a logic bug, not a type error) — worth keeping in mind that a
clean compile won't mean every route actually goes where it should.

Spots worth double-checking first against real compiler output if
`cargo build` does complain:

- Every closure captured into a `WebViewBuilder` handler here assumes it
  only ever runs on the GTK main thread and doesn't need to be `Send`
  (hence plain `Rc<RefCell<AppState>>`, no `Arc`/`Mutex`). If a `Send`
  bound trips somewhere, swap `Rc`/`RefCell` for `Arc`/`Mutex` in
  `state.rs` and the handler closures that capture it.
- `with_document_title_changed_handler`'s closure signature
  (`Fn(String)`) is inferred from a changelog note rather than seen
  directly in a current code example — lowest-confidence single call in
  the codebase.

`background.rs`'s use of `gtk::FileChooserNative` (`NativeDialogExt::run()`
for the blocking call, `FileChooserExt::filename()`/`add_filter()`) was
checked directly against the `gtk` 0.18 source on docs.rs, including the
exact `FileChooserNative::new(...)` argument order and that
`tao::platform::unix::WindowExtUnix::gtk_window()` is the right way to
get a parent window handle from tao's `Window` — same trait this crate
already uses for `default_vbox()`. Reasonably high confidence, but it's
the one part of this codebase exercising GTK dialog/file APIs rather
than wry's webview APIs, so it's worth a close look in the first build
too.

Everything else (custom protocol, IPC, navigation handler, bounds/Rect,
`build_gtk`, `go_back`/`go_forward`/`can_go_back`/`can_go_forward`,
`with_incognito`, `NewWindowResponse`, `with_devtools` + the `devtools`
cargo feature and its interaction with the native right-click context
menu) was checked against current docs.rs pages, the `wry` changelog, or
official examples.
