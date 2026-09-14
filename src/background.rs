// Custom browser background: lets the user pick a local image to show,
// blurred, behind SCIFI's glass toolbar and its own internal pages. Uses
// a native GTK file picker — no image-decoding dependency needed, since
// the file is copied byte-for-byte and served back as-is, never
// transcoded or resized by SCIFI itself.

use std::fs;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{ApplicationWindow, FileChooserAction, FileChooserNative, FileFilter, ResponseType};

/// Resolves (and creates, if needed) SCIFI's per-user data directory:
/// $XDG_DATA_HOME/scifi, or ~/.local/share/scifi if that's unset. No
/// `dirs` crate dependency — this is the one on-disk path SCIFI needs.
fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let dir = base.join("scifi");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Finds the current background file, if one has been set — whatever
/// extension it was originally copied in with. There's only ever one:
/// `set_from_path` removes any previous file (which may have had a
/// different extension) before writing a new one.
pub fn current_path() -> Option<PathBuf> {
    let dir = data_dir()?;
    fs::read_dir(&dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name();
        name.to_str()?
            .starts_with("background.")
            .then(|| entry.path())
    })
}

/// Content-type to serve the current background as, guessed from its
/// extension — no image-sniffing dependency, since SCIFI never decodes
/// or transcodes the file, only copies and re-serves the original bytes.
pub fn current_content_type() -> Option<&'static str> {
    let path = current_path()?;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    })
}

/// Reads the current background's bytes, if one is set.
pub fn read_current() -> Option<Vec<u8>> {
    fs::read(current_path()?).ok()
}

/// Shows a native "choose an image" dialog (using the desktop's file
/// portal where available, so this also works correctly in a sandboxed
/// environment) and, if the user picks a file, copies it into SCIFI's
/// data directory as the new background. Returns whether it changed.
pub fn pick_and_set(parent: &ApplicationWindow) -> bool {
    let filter = FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_mime_type("image/*");

    let dialog = FileChooserNative::new(
        Some("Choose a background image"),
        Some(parent),
        FileChooserAction::Open,
        Some("Choose"),
        Some("Cancel"),
    );
    dialog.add_filter(&filter);

    let chosen = if dialog.run() == ResponseType::Accept {
        dialog.filename()
    } else {
        None
    };

    match chosen {
        Some(source) => set_from_path(&source),
        None => false,
    }
}

fn set_from_path(source: &std::path::Path) -> bool {
    let Some(dir) = data_dir() else {
        return false;
    };
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("img")
        .to_ascii_lowercase();

    // Drop any previous background first — it may have had a different
    // extension, so it wouldn't just be overwritten by the copy below.
    clear();

    let dest = dir.join(format!("background.{ext}"));
    fs::copy(source, &dest).is_ok()
}

/// Removes the current background file, if any, reverting SCIFI to its
/// default gradient.
pub fn clear() {
    if let Some(path) = current_path() {
        let _ = fs::remove_file(path);
    }
}
