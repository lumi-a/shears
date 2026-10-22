//! Graph model and analysis for shears, a `KubeJS` recipe-graph tool.
//!
//! No filesystem access and no threads unless the `fs` and `parallel` features are
//! enabled. With both off it builds for `wasm32-unknown-unknown`.

// This lint only fires when this crate's own `lib test` harness compiles (the harness
// is itself a binary, unlike the plain `lib` build), so it's gated on `cfg(test)`..
#![cfg_attr(
    test,
    expect(
        unused_crate_dependencies,
        reason = "dev-dependencies are split unevenly across this package's test \
                  binaries (one needs tar/zstd/tempfile, none of them need every \
                  dependency this crate declares), and this includes the \
                  self-referencing dev-dependency that only exists to enable the `fs` \
                  feature for integration tests."
    )
)]

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
