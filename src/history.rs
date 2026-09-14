// Browsing history for normal (non-incognito) tabs, persisted to disk.
// Incognito tabs never call into this module at all — see
// state::AppState::incognito_session_log for their session-only,
// never-written-to-disk equivalent.
//
// Format is newline-delimited JSON (one compact JSON object per visit,
// append-only) rather than a database: no new dependency (serde_json is
// already here), crash-safe by construction (a torn last line just fails
// to parse and is skipped, nothing else is corrupted), and easy to
// inspect or clear by hand. Writes happen on a small background thread
// so a page finishing a load never blocks the GTK main thread on disk
// I/O.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    /// Unix seconds.
    pub visited_at: u64,
}

/// Same data directory background.rs uses — $XDG_DATA_HOME/scifi, or
/// ~/.local/share/scifi if that's unset.
fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let dir = base.join("scifi");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn history_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("history.ndjson"))
}

fn writer() -> &'static Sender<HistoryEntry> {
    static WRITER: OnceLock<Sender<HistoryEntry>> = OnceLock::new();
    WRITER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<HistoryEntry>();
        std::thread::spawn(move || {
            for entry in rx {
                let Some(path) = history_path() else { continue };
                let Ok(json) = serde_json::to_string(&entry) else {
                    continue;
                };
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
                    let _ = writeln!(file, "{json}");
                }
            }
        });
        tx
    })
}

/// Records a visit. Non-blocking — handed off to the background writer.
/// SCIFI's own internal pages (scifi://...) are never logged.
pub fn record_visit(url: &str, title: &str) {
    if url.starts_with("scifi://") {
        return;
    }
    let entry = HistoryEntry {
        url: url.to_string(),
        title: title.to_string(),
        visited_at: now_unix_secs(),
    };
    let _ = writer().send(entry);
}

/// Loads every entry, most recent first. Capped at `limit` so a very
/// long-lived history doesn't mean shipping an ever-growing JSON blob to
/// the Settings page on every sync.
pub fn load_recent(limit: usize) -> Vec<HistoryEntry> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(file) = fs::File::open(&path) else {
        return Vec::new();
    };
    let mut entries: Vec<HistoryEntry> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

pub fn clear() {
    if let Some(path) = history_path() {
        let _ = fs::remove_file(path);
    }
}

/// Current unix time in seconds — exposed so callers building a
/// `HistoryEntry` that *won't* go through `record_visit` (i.e. the
/// in-memory incognito session log in state.rs, which never calls this
/// module's disk-writing path) can still stamp a real time.
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
