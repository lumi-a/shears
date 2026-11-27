//! Criterion benchmark for analyzing the Monifactory fixture: applying a rule file and
//! computing what's transitively required.
//!
//! `cargo bench -p shears-core` prints the percentage change against the previous run's saved
//! baseline.
//!
//! Ingest runs once beforehand and isn't timed, see the `ingest` benchmark for that.

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

use criterion::{Criterion, criterion_group, criterion_main};
use shears_core::analysis::edges::{EdgeStates, NodeAttrs};
use shears_core::analysis::transitively_required::transitively_required;
use shears_core::ingest;
use shears_core::rules::RuleSet;

#[path = "../tests/support/helpers.rs"]
mod support;

/// A small rule file whose required items reach about half of Monifactory's items.
const RULES_TOML: &str = r#"
[[item-rule]]
reason = "benchmark: every ingot"
filter = { id = "/.*:.*_ingot/" }
action = { required = true }

[[item-rule]]
reason = "benchmark: some tools"
filter = { id = "/gtceu:.*_(wrench|hammer|file|saw)/" }
action = { required = true }
"#;

/// Benchmarks parsing and applying [`RULES_TOML`], then the transitive-requirement pass.
fn bench_analyze(c: &mut Criterion) {
    let files = support::read_fixture_in_memory(&support::archive_path());
    let (graph, _report) = ingest::ingest(&files, ingest::DEFAULT_SHAPES_TOML)
        .expect("the default parsing.toml parses");

    let mut group = c.benchmark_group("analyze");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("monifactory", |b| {
        b.iter(|| {
            let rules =
                RuleSet::parse(criterion::black_box(RULES_TOML)).expect("the rules parse");
            let mut attrs = NodeAttrs::defaults(&graph);
            let mut edges = EdgeStates::defaults(&graph);
            rules.apply(&graph, &mut attrs, &mut edges, 0);
            transitively_required(&graph, &attrs, &edges)
        });
    });
    group.finish();
}

criterion_group!(benches, bench_analyze);
criterion_main!(benches);
