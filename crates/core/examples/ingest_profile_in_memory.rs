//! A `samply record` target for `ingest()` invocation for Monifactory.
//!
//! This one reads from the extracted archive instead of from disk, which is faster.
//!
//! Doesn't loop, to avoid cache warmup.
//!
//! ```sh
//! cargo build --release --example ingest_profile_in_memory -p shears-core
//! samply record -- target/release/examples/ingest_profile_in_memory
//! ```

#![expect(unused_crate_dependencies, reason = "profiling target, not a binary")]

use std::time::Instant;

use shears_core::ingest;

/// The archive's one top-level directory present as a prefix on every entry's path.
#[path = "../tests/support/mod.rs"]
mod support;

#[expect(
    clippy::print_stdout,
    clippy::use_debug,
    reason = "profiling may print"
)]
fn main() {
    let archive_path = support::archive_path();

    // Everything from here down is the measured region.
    let start = Instant::now();

    let files = support::read_fixture_in_memory(&archive_path);
    let decode_elapsed = start.elapsed();

    let (graph, report) = ingest::ingest(&files);

    let total_elapsed = start.elapsed();

    println!("files read: {}", files.len());
    println!("decompress + walk archive: {decode_elapsed:?}");
    println!("decompress + walk + ingest total: {total_elapsed:?}");
    println!("items: {}", graph.items().len());
    println!("recipes: {}", graph.recipes().len());
    println!("diagnostics: {}", report.diagnostics.len());
}
