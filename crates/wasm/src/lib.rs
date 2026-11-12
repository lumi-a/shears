//! WASM bindings for shears.
//!
//! Build with `wasm-bindgen`.

#![expect(
    clippy::missing_inline_in_public_items,
    reason = "wasm-bindgen exported functions can't be #[inline]: it's ignored on externally \
              exported functions, and emits its own warning if added"
)]

use shears_core::archive;
use shears_core::ingest::find_root_directory;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

/// A quick look at what [`unpack_archive`] found, before we start ingest.
#[derive(Debug)]
#[wasm_bindgen]
pub struct ArchiveSummary {
    /// The wrapper directory the export sat in.
    ///
    /// Empty if the archive's top level was already the export root.
    root_directory: String,
    /// How many files the export contains starting from root (directories not counted).
    file_count: u32,
    /// The combined size of every file's contents in bytes.
    total_bytes: u32,
}

#[wasm_bindgen]
#[expect(
    clippy::missing_const_for_fn,
    reason = "wasm-bindgen exported functions can't be const"
)]
impl ArchiveSummary {
    /// The wrapper directory the export sat in.
    ///
    /// Empty if the archive's top level was already the export root.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn root_directory(&self) -> String {
        self.root_directory.clone()
    }

    /// How many files the export contains starting from root (directories not counted).
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn file_count(&self) -> u32 {
        self.file_count
    }

    /// The combined size of every file's contents in bytes.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn total_bytes(&self) -> u32 {
        self.total_bytes
    }
}

/// Unpacks `bytes` archive, finds the export root inside it, and summarizes what it found.
///
/// # Errors
///
/// If `bytes` isn't a recognized archive format, the archive itself is corrupt, or it
/// doesn't contain exactly one export. The error is not yet a structured value. TODO:
/// Should it (*can* it) be?
#[wasm_bindgen]
pub fn unpack_archive(bytes: &[u8]) -> Result<ArchiveSummary, JsValue> {
    let unpacked =
        archive::unpack(bytes).map_err(|error| JsValue::from_str(&error.to_string()))?;
    let (root_directory, files) = find_root_directory::strip(unpacked)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;

    let file_count = u32::try_from(files.len()).unwrap_or(u32::MAX);
    let total_bytes_usize: usize = files.iter().map(|file| file.bytes.len()).sum();
    let total_bytes = u32::try_from(total_bytes_usize).unwrap_or(u32::MAX);

    Ok(ArchiveSummary {
        root_directory,
        file_count,
        total_bytes,
    })
}
