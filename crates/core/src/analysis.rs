//! Analysis of the graph state.
//!
//! Annotate first, then compute (edge states, liveness, byproducts, open competitions).

pub mod adjacency;
pub mod edges;
pub mod liveness;

#[cfg(test)]
mod proptest_support;
