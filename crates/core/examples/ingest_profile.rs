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

use core::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

use sha2::{Digest as _, Sha256};
use shears_core::{ingest, io};

/// Extracts `archive_path` (a `tar`+`zstd` stream) into a cache directory under
/// `target/profile-fixture-cache/<hash of archive bytes>/`.
///
/// Extraction happens into a `pid`-suffixed staging directory first, published via an atomic
/// rename, so a run that gets killed mid-extraction can never be mistaken for a valid cache
/// hit by a later run.
#[expect(clippy::print_stdout, reason = "profiling should print")]
fn cached_export_root(archive_path: &Path) -> PathBuf {
    let archive_bytes =
        fs::read(archive_path).expect("the committed Monifactory fixture exists");
    let digest = Sha256::digest(&archive_bytes);
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hash, "{byte:02x}").expect("writing a hex digit to a String never fails");
    }

    let cache_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/profile-fixture-cache");
    let cache_dir = cache_root.join(&hash);
    let export_root = cache_dir.join("monifactory-0.13.8-normalmode");
    if export_root.is_dir() {
        println!("fixture cache: hit ({})", cache_dir.display());
        return export_root;
    }

    println!("fixture cache: miss, extracting to {}", cache_dir.display());
    fs::create_dir_all(&cache_root).expect("creating the cache root");
    let staging = cache_root.join(format!("{hash}.staging-{}", process::id()));
    fs::remove_dir_all(&staging).unwrap_or(());
    fs::create_dir_all(&staging).expect("creating the staging directory");

    let archive = File::open(archive_path).expect("the committed Monifactory fixture exists");
    let decoder = zstd::Decoder::new(archive).expect("a valid zstd stream");
    tar::Archive::new(decoder)
        .unpack(&staging)
        .expect("a valid tar archive");

    if let Err(error) = fs::rename(&staging, &cache_dir) {
        // Another concurrent run may have already published this exact hash. Our copy is
        // redundant in that case, otherwise this really is a problem.
        assert!(
            cache_dir.is_dir(),
            "publishing the extracted cache dir: {error}"
        );
        fs::remove_dir_all(&staging).unwrap_or(());
    }
    export_root
}

#[expect(
    clippy::print_stdout,
    clippy::use_debug,
    reason = "profiling may print"
)]
fn main() {
    let archive_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/monifactory-0.13.8-normalmode.tar.zst");
    let root = cached_export_root(&archive_path);

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
    println!("diagnostics: {}", report.diagnostics.len());
}
