//! Graph model and analysis for shears, a `KubeJS` recipe-graph tool.
//!
//! No filesystem access and no threads unless the `fs` and `parallel` features are enabled.
//! With both off it builds for `wasm32-unknown-unknown`.

/// The graph, and its constituent Items and Recipes.
pub mod model;
