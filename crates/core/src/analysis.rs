//! Annotate first, then compute
//! (edge states, liveness, byproducts, open competitions).

/// Derived per-edge and per-node state, and the reasons behind it.
pub mod edges;

/// Lookups derived from a `Graph`. Not serialized, rebuilt after loading instead.
pub mod adjacency;
