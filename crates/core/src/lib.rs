//! Graph model and analysis for shears, a `KubeJS` recipe-graph tool.
//!
//! No filesystem access and no threads unless the `fs` and `parallel` features are enabled.
//! With both off it builds for `wasm32-unknown-unknown`.

/// The graph, and its constituent Items and Recipes.
pub mod model;

/// Analysis of the graph state.
pub mod analysis;

/// Turns a `/kubejs export` into a [`model::Graph`].
///
/// Problems turn into [`ingest::Diagnostic`]s in the returned [`ingest::ParseReport`].
///
/// Tags and the item/fluid registries are resolved first.
/// Every recipe file can then be parsed to real ids without incomplete drafts.
pub mod ingest;

/// Reads a raw export filesystem directory into [`ingest::RawFile`]s.
///
/// Behind the `fs` feature.
#[cfg(feature = "fs")]
pub mod io;

/// A tiny textual DSL for hand-written test graphs. Test-only.
#[cfg(test)]
mod graph_dsl;
