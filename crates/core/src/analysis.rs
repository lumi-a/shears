//! Analysis of the graph state.
//!
//! Annotate first, then compute (edge states, transitively-required items, byproducts,
//! open competitions).

pub mod adjacency;
pub mod edges;
pub mod transitively_required;

#[cfg(test)]
mod proptest_support;
