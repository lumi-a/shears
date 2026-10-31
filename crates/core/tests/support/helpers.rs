//! Shared helpers for examples, benches and tests that need the Monifactory fixture.

#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) over pub here is intentional (see unreachable_pub), even though this \
              module's own privacy makes it stylistically redundant"
)]
#![expect(
    dead_code,
    reason = "shared across several examples/benches via #[path = ...]; no single consumer \
              uses every item, so each compiles with part of this module unused"
)]

use core::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process;

use sha2::{Digest as _, Sha256};
use shears_core::ingest::RawFile;

/// The archive's one top-level directory, present as a prefix on every entry's path.
const FIXTURE_ROOT: &str = "monifactory-0.13.8-normalmode";

/// Path to the committed Monifactory fixture archive.
pub(crate) fn archive_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/monifactory-0.13.8-normalmode.tar.zst")
}

/// Decompresses and walks `archive_path`'s entries directly into [`RawFile`]s, with no
/// filesystem access beyond reading the one archive file.
pub(crate) fn read_fixture_in_memory(archive_path: &Path) -> Vec<RawFile> {
    let archive_bytes = fs::read(archive_path).expect("the committed fixture exists");
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
    files
}

/// Extracts `archive_path` (a `tar`+`zstd` stream) into a cache directory under
/// `target/profile-fixture-cache/<hash of archive bytes>/`, reusing it on a cache hit instead
/// of re-extracting.
#[expect(
    clippy::print_stdout,
    reason = "a profiling/benchmark target's whole purpose is printing what it measured"
)]
pub(crate) fn cached_export_root(archive_path: &Path) -> PathBuf {
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
