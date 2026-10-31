//! Criterion benchmark for `ingest()` pipeline against the Monifactory fixture.
//!
//! `cargo bench -p shears-core` prints the percentage change against the previous run's saved
//! baseline.
//!
//! Sample size and measurement time are both widened well past criterion's defaults, because
//! `ingest()` takes relatively long by criterion's standards.

#![expect(
    unused_crate_dependencies,
    reason = "benchmark target, not a real binary"
)]
#![expect(
    missing_docs,
    reason = "criterion_group!/criterion_main! generate undocumented items we don't control"
)]
#![expect(
    unused_results,
    reason = "criterion::BenchmarkGroup's setters return &mut Self for chaining"
)]

use core::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use shears_core::ingest;

#[path = "../tests/support/helpers.rs"]
mod support;

/// Benchmarks `ingest()` over the real Monifactory fixture.
fn bench_ingest(c: &mut Criterion) {
    let files = support::read_fixture_in_memory(&support::archive_path());

    let mut group = c.benchmark_group("ingest");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(25));
    group.bench_function("monifactory", |b| {
        b.iter(|| ingest::ingest(criterion::black_box(&files)));
    });
    group.finish();
}

criterion_group!(benches, bench_ingest);
criterion_main!(benches);
