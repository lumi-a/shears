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

use std::fs;
use std::io::Read as _;
use std::path::PathBuf;
use std::time::Instant;

use shears_core::ingest::{self, RawFile};

/// The archive's one top-level directory present as a prefix on every entry's path.
const FIXTURE_ROOT: &str = "monifactory-0.13.8-normalmode";

#[expect(
    clippy::print_stdout,
    clippy::use_debug,
    reason = "profiling may print"
)]
fn main() {
    let archive_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/monifactory-0.13.8-normalmode.tar.zst");
    let archive_bytes = fs::read(&archive_path).expect("the committed fixture exists");

    // Everything from here down is the measured region.
    let start = Instant::now();

    let decoder = zstd::Decoder::new(archive_bytes.as_slice()).expect("a valid zstd stream");
    let mut archive = tar::Archive::new(decoder);
    let mut files = Vec::new();
    for raw_entry in archive.entries().expect("a readable tar stream") {
        let mut entry = raw_entry.expect("a readable tar entry");
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let full_path = entry.path().expect("a valid entry path").into_owned();
        let relative = full_path.strip_prefix(FIXTURE_ROOT).unwrap_or(&full_path);
        let path = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let mut bytes = Vec::new();
        let _: usize = entry
            .read_to_end(&mut bytes)
            .expect("reading the entry's contents");
        files.push(RawFile { path, bytes });
    }
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
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
