//! Graph model and analysis for shears, a `KubeJS` recipe-graph tool.
//!
//! No filesystem access and no threads unless the `fs` and `parallel` features are
//! enabled. With both off it builds for `wasm32-unknown-unknown`.

pub mod analysis;
pub mod model;
pub mod rules;

pub mod ingest;

/// Reads a raw export filesystem directory into [`ingest::RawFile`]s.
///
/// Behind the `fs` feature.
#[cfg(feature = "fs")]
pub mod io;

/// A tiny textual DSL for hand-written test graphs. Test-only.
#[cfg(test)]
mod graph_dsl;
