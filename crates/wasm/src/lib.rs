//! WASM bindings for shears.
//!
//! Build with `wasm-bindgen`.
//!
//! Big data stays in WASM memory. JS hands over the archive's bytes just once and only ever
//! gets small results (counts, the parse report) back.

#![expect(
    clippy::missing_inline_in_public_items,
    reason = "wasm-bindgen exported functions can't be #[inline]: it's ignored on externally \
              exported functions, and emits its own warning if added"
)]

use shears_core::archive;
use shears_core::ingest::{self, find_root_directory, ParseReport, RawFile};
use shears_core::model::Graph;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

/// An error for JS, from anything displayable.
///
/// Just its message for now. Should be a structured value at some point.
fn js_error(error: &impl ToString) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// One uploaded export, consisting of its files and the graph ingested from them.
///
/// The files stay in WASM so a changed parsing.toml can re-ingest without another upload.
#[derive(Debug)]
#[wasm_bindgen]
pub struct Session {
    /// The wrapper directory the export sat in, empty if there was no wrapper.
    root_directory: String,
    /// The export's files, paths relative to its root.
    files: Vec<RawFile>,
    /// The graph from the latest successful ingest.
    graph: Graph,
    /// The parse report from the latest successful ingest.
    report: ParseReport,
}

#[wasm_bindgen]
#[expect(
    clippy::missing_const_for_fn,
    reason = "wasm-bindgen exported functions can't be const"
)]
impl Session {
    /// Unpacks `bytes` (a `.zip` or `.tar.zst`), finds the export inside, and ingests it with
    /// `shapes_toml` (parsing.toml's contents).
    ///
    /// # Errors
    ///
    /// If `bytes` isn't a recognized or intact archive, doesn't contain an export, or
    /// `shapes_toml` doesn't parse.
    pub fn open(bytes: &[u8], shapes_toml: &str) -> Result<Self, JsValue> {
        let unpacked = archive::unpack(bytes).map_err(|error| js_error(&error))?;
        let (root_directory, files) =
            find_root_directory::strip(unpacked).map_err(|error| js_error(&error))?;
        let (graph, report) =
            ingest::ingest(&files, shapes_toml).map_err(|error| js_error(&error))?;
        Ok(Self {
            root_directory,
            files,
            graph,
            report,
        })
    }

    /// Ingests the same files again with a new `shapes_toml`.
    ///
    /// # Errors
    ///
    /// If `shapes_toml` doesn't parse. The previous graph and report are kept then.
    pub fn reingest(&mut self, shapes_toml: &str) -> Result<(), JsValue> {
        let (graph, report) =
            ingest::ingest(&self.files, shapes_toml).map_err(|error| js_error(&error))?;
        self.graph = graph;
        self.report = report;
        Ok(())
    }

    /// The wrapper directory the export sat in, empty if there was none.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn root_directory(&self) -> String {
        self.root_directory.clone()
    }

    /// How many files the export has.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// How many items and fluids the graph has.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.graph.items().len()
    }

    /// How many recipes the graph has.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn recipe_count(&self) -> usize {
        self.graph.recipes().len()
    }

    /// The parse report as JSON (mirrored by `ParseReport` in `web/src/session.ts`).
    ///
    /// # Errors
    ///
    /// Never in practice: the report is plain strings and numbers.
    pub fn report_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.report).map_err(|error| js_error(&error))
    }
}
