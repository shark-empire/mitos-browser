// SCIFI's navigation policy: a scheme allowlist that always applies, plus
// a host blocklist that applies only when tracking protection is on
// (toggleable from the Settings page — see state::AppState::tracking_protection).
//
// This is a deliberately small, dependency-free URL parser — good enough
// to pull a scheme and host out for policy checks, not a spec-compliant
// one. Unusual forms (IPv6 literal hosts, userinfo containing `@` in the
// path, etc.) may not be classified correctly; pull in the `url` crate if
// you need RFC 3986-correct parsing.

/// Why a navigation was blocked. Kept as an enum (rather than a bare
/// string) so callers can distinguish "this was a tracking block" (which
/// should count toward the Settings page's stat) from "this was a scheme
/// block" (a security boundary, not a tracking one) without string
/// matching.
#[derive(Clone, Copy)]
pub enum BlockReason {
    NoScheme,
    DisallowedScheme,
    TrackerHost,
}

impl BlockReason {
    pub fn is_tracker(&self) -> bool {
        matches!(self, BlockReason::TrackerHost)
    }
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            BlockReason::NoScheme => "no scheme",
            BlockReason::DisallowedScheme => "disallowed scheme",
            BlockReason::TrackerHost => "known tracker/ad domain",
        };
        f.write_str(label)
    }
}

/// Result of evaluating whether a navigation should be allowed to proceed.
pub enum Verdict {
    Allow,
    Block(BlockReason),
}

/// Schemes SCIFI will navigate to, always, regardless of the tracking
/// protection toggle. Notably absent: `file:` (no arbitrary local
/// filesystem access from a page or a pasted address), `javascript:` (no
/// address-bar self-XSS), and `data:` (wry already refuses it for
/// top-level navigation, but this doesn't rely on that alone).
const ALLOWED_SCHEMES: &[&str] = &["https", "http", "scifi"];

/// A small starter list of known ad/tracking hosts, blocked at the
/// navigation layer when tracking protection is on. Intentionally short
/// and illustrative rather than exhaustive — swap in a maintained block
/// list (e.g. one of the public hosts-file projects) for real-world
/// coverage. `pub` so the Settings page can display what's in effect.
pub const BLOCKED_HOSTS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "google-analytics.com",
    "googletagmanager.com",
    "adnxs.com",
    "adsrvr.org",
    "scorecardresearch.com",
    "outbrain.com",
    "taboola.com",
    "criteo.com",
    "amazon-adsystem.com",
    "facebook.net",
];

/// Classifies a fully-formed URL a webview is about to navigate to.
/// `tracking_protection` gates the host blocklist only — the scheme
/// allowlist is a security boundary and always applies.
pub fn classify(target: &str, tracking_protection: bool) -> Verdict {
    let scheme = match target.split_once(':') {
        Some((scheme, _)) => scheme.to_ascii_lowercase(),
        None => return Verdict::Block(BlockReason::NoScheme),
    };

    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Verdict::Block(BlockReason::DisallowedScheme);
    }

    if tracking_protection && (scheme == "http" || scheme == "https") {
        if let Some(host) = extract_host(target) {
            let blocked = BLOCKED_HOSTS
                .iter()
                .any(|blocked| host == *blocked || host.ends_with(&format!(".{blocked}")));
            if blocked {
                return Verdict::Block(BlockReason::TrackerHost);
            }
        }
    }

    Verdict::Allow
}

/// Outcome of resolving whatever the user typed in the address bar.
pub enum AddressResolution {
    /// Nothing was typed — do nothing, no notice.
    Empty,
    /// Resolved to a URL that's allowed to load.
    Url(String),
    /// Doesn't look like an address, and there's no default search engine
    /// wired up yet, so it's rejected rather than silently guessed at.
    NotAnAddress,
    /// Looked like an address but resolved to something the navigation
    /// policy blocks.
    Blocked(BlockReason),
}

/// Resolves what the user typed in the address bar per the same policy
/// `classify` applies to in-page navigation.
pub fn resolve_address_bar_input(input: &str, tracking_protection: bool) -> AddressResolution {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return AddressResolution::Empty;
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else if looks_like_domain(trimmed) {
        format!("https://{trimmed}")
    } else {
        return AddressResolution::NotAnAddress;
    };

    match classify(&candidate, tracking_protection) {
        Verdict::Allow => AddressResolution::Url(candidate),
        Verdict::Block(reason) => AddressResolution::Blocked(reason),
    }
}

fn looks_like_domain(s: &str) -> bool {
    s.contains('.') && !s.contains(' ')
}

fn extract_host(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let host_part = after_scheme.split(['/', '?', '#']).next()?;
    let host = host_part.rsplit('@').next()?; // drop userinfo if present
    let host = host.split(':').next()?; // drop port if present
    Some(host.to_ascii_lowercase())
}
