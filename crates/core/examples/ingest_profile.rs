//! A `samply record` target for `ingest()` invocation for Monifactory.
//!
//! Reads from disk, for sake of realism. For a WASM-shaped alternative that just reads
//! an archive, see `ingest_profile_in_memory` instead.
//!
//! Doesn't loop, to avoid cache warmup.
//!
//! The fixture is extracted into a cached directory if it hasn't been yet (validated by
//! cryptographic hash). This extraction is not included in the run, but reading back from the
//! filesystem then _is_.
//!
//! ```sh
//! cargo build --release --example ingest_profile -p shears-core
//! samply record -- target/release/examples/ingest_profile
//! ```

#![expect(
    unused_crate_dependencies,
    reason = "profiling target, not a real binary"
)]

use std::time::Instant;

use shears_core::{ingest, io};
#[path = "../tests/support/mod.rs"]
mod support;

#[expect(
    clippy::print_stdout,
    clippy::use_debug,
    reason = "profiling may print"
)]
fn main() {
    let root = support::cached_export_root(&support::archive_path());

    // Everything from here down is measured.
    let start = Instant::now();

    let files = io::read_export_dir(&root).expect("reading the extracted export");
    let read_elapsed = start.elapsed();

    let (graph, report) = ingest::ingest(&files);

    let total_elapsed = start.elapsed();

    println!("files read: {}", files.len());
    println!("read_export_dir: {read_elapsed:?}");
    println!("read + ingest total: {total_elapsed:?}");
    println!("items: {}", graph.items().len());
    println!("recipes: {}", graph.recipes().len());
    println!("tags: {}", graph.tags().len());
    println!("diagnostics: {}", report.diagnostics.len());
}
