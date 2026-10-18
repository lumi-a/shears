use std::fs;
use std::io::{Error, Result};
use std::path::Path;

use walkdir::WalkDir;

use crate::ingest::RawFile;

/// Reads every file under `root` into a [`RawFile`], with `path` relative to `root`
/// and using `/` separators regardless of platform. Visits files in a deterministic
/// (sorted) order.
///
/// # Errors
///
/// Returns an error if `root` can't be walked, or if any file under it can't be read.
#[expect(
    clippy::missing_inline_in_public_items,
    reason = "big top-level function, not an inlining candidate"
)]
pub fn read_export_dir(root: &Path) -> Result<Vec<RawFile>> {
    let mut files = Vec::new();

    for walked in WalkDir::new(root).sort_by_file_name() {
        let entry = walked.map_err(Error::other)?;
        if entry.file_type().is_dir() {
            continue;
        }

        let relative = entry.path().strip_prefix(root).map_err(Error::other)?;
        let path = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(entry.path())?;
        files.push(RawFile { path, bytes });
    }

    Ok(files)
}
