//! Command-line frontend for shears.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use shears_core::analysis::edges::{EdgeStates, NodeAttrs};
use shears_core::analysis::transitively_required::transitively_required;
use shears_core::model::Graph;
use shears_core::rules::{RuleError, RuleSet};

/// shears: Eliminate unneeded recipes in `KubeJS` Minecraft Modpacks.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Which operation to run.
    #[command(subcommand)]
    command: Command,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Applies rules to a graph and prints a summary of what's transitively required.
    Analyze {
        /// Path to the normalized graph produced by ingest (`graph.json`).
        ///
        /// TODO: Would it not be a good idea to intsead pass a directory here, which is
        /// then, say, hashed to check for staleness (do most filesystems support fast
        /// cryptographic hashing of a directory? We could do Merkle tree to at least save
        /// us from reading all the JSON data, but still not sure if that'd be fast
        /// enough), and if stale, graph.json is regenerated, and if not stale, it is read
        /// from disk?
        graph: PathBuf,
        /// A rules file to apply.
        #[arg(long = "rules")]
        rules: Vec<PathBuf>,
    },
}

/// What went wrong running [`Command::Analyze`].
#[derive(Debug, thiserror::Error)]
enum AnalyzeError {
    /// A file couldn't be read at all.
    #[error("couldn't read {path}: {source}")]
    Read {
        /// The file that couldn't be read.
        path: PathBuf,
        /// The underlying IO error.
        #[source]
        source: io::Error,
    },
    /// The graph file's contents weren't a valid [`Graph`].
    #[error("{path}: invalid graph: {source}")]
    InvalidGraph {
        /// The graph file that failed to parse.
        path: PathBuf,
        /// Why it failed to parse.
        #[source]
        source: serde_json::Error,
    },
    /// A rules file's contents weren't a valid [`RuleSet`].
    #[error("{path}: {source}")]
    InvalidRules {
        /// The rules file that failed to parse.
        path: PathBuf,
        /// Why it failed to parse.
        #[source]
        source: Box<RuleError>,
    },
    /// More rules files were passed than [`RuleSet::apply`]'s `file` argument can index.
    #[error("too many rules files (more than {} aren't supported)", u16::MAX)]
    TooManyRulesFiles,
}

/// Counts the analysis produced, for [`print_summary`].
#[derive(Debug)]
struct Summary {
    /// Total items and fluids in the graph.
    item_count: usize,
    /// Total recipes in the graph.
    recipe_count: usize,
    /// Items marked required directly, by a preset or a rule.
    required_count: usize,
    /// Items that ended up transitively required.
    transitively_required_item_count: usize,
    /// Recipes that propagate at least one transitively required output.
    transitively_required_recipe_count: usize,
    /// Warnings from every rules file that was applied, prefixed with that file's path.
    warnings: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyze { graph, rules } => match analyze(&graph, &rules) {
            Ok(summary) => {
                print_summary(&summary);
                ExitCode::SUCCESS
            }
            Err(error) => {
                report_error(&error);
                ExitCode::FAILURE
            }
        },
    }
}

/// Reads `graph_path` and every path in `rules_paths` (in order), applies them, and summarizes
/// the result.
///
/// # Errors
///
/// If any file can't be read, the graph isn't valid JSON, a rules file isn't valid, or more
/// than `u16::MAX` rules files were passed.
fn analyze(graph_path: &Path, rules_paths: &[PathBuf]) -> Result<Summary, AnalyzeError> {
    let graph = read_graph(graph_path)?;

    let mut attrs = NodeAttrs::defaults(&graph);
    let mut edges = EdgeStates::defaults(&graph);
    let mut warnings = Vec::new();

    for (position, rules_path) in rules_paths.iter().enumerate() {
        let file =
            u16::try_from(position).map_err(|_overflow| AnalyzeError::TooManyRulesFiles)?;
        let rules = read_rules(rules_path)?;
        rules.apply(&graph, &mut attrs, &mut edges, file);
        warnings.extend(
            rules
                .warnings()
                .iter()
                .map(|warning| format!("{}: {warning}", rules_path.display())),
        );
    }

    let live = transitively_required(&graph, &attrs, &edges);

    Ok(Summary {
        item_count: graph.items().len(),
        recipe_count: graph.recipes().len(),
        required_count: attrs.required_items().count(),
        transitively_required_item_count: live.transitively_required_item_count(),
        transitively_required_recipe_count: live.transitively_required_recipe_count(),
        warnings,
    })
}

/// Reads and parses `path` as a [`Graph`].
///
/// # Errors
///
/// If `path` can't be read, or its contents aren't a valid [`Graph`].
fn read_graph(path: &Path) -> Result<Graph, AnalyzeError> {
    let text = fs::read_to_string(path).map_err(|source| AnalyzeError::Read {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| AnalyzeError::InvalidGraph {
        path: path.to_owned(),
        source,
    })
}

/// Reads and parses `path` as a [`RuleSet`].
///
/// # Errors
///
/// If `path` can't be read, or its contents aren't a valid [`RuleSet`].
fn read_rules(path: &Path) -> Result<RuleSet, AnalyzeError> {
    let text = fs::read_to_string(path).map_err(|source| AnalyzeError::Read {
        path: path.to_owned(),
        source,
    })?;
    RuleSet::parse(&text).map_err(|source| AnalyzeError::InvalidRules {
        path: path.to_owned(),
        source: Box::new(source),
    })
}

/// Prints `summary` to stdout.
#[expect(
    clippy::print_stdout,
    reason = "a CLI's whole purpose is printing its result"
    // TODO: Aren't there, like, `clap::log` functions like in python's Click?
)]
fn print_summary(summary: &Summary) {
    println!(
        "{} items, {} recipes",
        summary.item_count, summary.recipe_count
    );
    println!(
        "{} required directly, {} transitively required ({} recipes propagate)",
        summary.required_count,
        summary.transitively_required_item_count,
        summary.transitively_required_recipe_count
    );
    if summary.warnings.is_empty() {
        return;
    }
    println!("{} warning(s):", summary.warnings.len());
    for warning in &summary.warnings {
        println!("  {warning}");
    }
}

/// Prints `error` to stderr.
#[expect(
    clippy::print_stderr,
    reason = "no logging framework yet; stderr is where errors belong"
)]
fn report_error(error: &AnalyzeError) {
    eprintln!("error: {error}");
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::{CommandFactory as _, Parser as _};

    use super::{Cli, Command};

    #[test]
    fn analyze_accepts_a_graph_path_and_repeated_rules_flags() {
        let cli = Cli::parse_from([
            "shears",
            "analyze",
            "graph.json",
            "--rules",
            "presets/vanilla.toml",
            "--rules",
            "rules.toml",
        ]);

        let Command::Analyze { graph, rules } = cli.command;
        assert_eq!(graph, PathBuf::from("graph.json"));
        assert_eq!(
            rules,
            vec![
                PathBuf::from("presets/vanilla.toml"),
                PathBuf::from("rules.toml"),
            ]
        );
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
