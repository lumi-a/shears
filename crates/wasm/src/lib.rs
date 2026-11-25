//! WASM bindings for shears.
//!
//! Build with `wasm-bindgen`.
//!
//! Big data stays in WASM memory. JS hands over the archive's bytes just once and only ever
//! gets small results (counts, the parse report) back.

#![expect(
    clippy::missing_inline_in_public_items,
    reason = "wasm-bindgen exported functions can't be #[inline]: it's ignored on externally \
              exported functions, and emits its own warning if added"
)]

mod details;
mod explore;

use std::collections::BTreeSet;

use serde::Serialize;
use shears_core::analysis::adjacency::Lookups;
use shears_core::analysis::edges::{EdgeStates, NodeAttrs};
use shears_core::analysis::transitively_required::{
    TransitivelyRequired, transitively_required,
};
use shears_core::archive;
use shears_core::ingest::{self, ParseReport, RawFile, find_root_directory};
use shears_core::model::Graph;
use shears_core::rules::RuleSet;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

/// An error for JS, from anything displayable.
///
/// Just its message for now. Should be a structured value at some point.
fn js_error(error: &impl ToString) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// The results of applying rules.toml to the current graph.
#[derive(Debug, Serialize)]
struct Analysis {
    /// Items marked required directly by a rule.
    required_count: usize,
    /// Items that ended up transitively required.
    transitively_required_item_count: usize,
    /// Recipes that propagate at least one transitively required output.
    transitively_required_recipe_count: usize,
    /// Things in the rules that parsed fine but are probably mistakes.
    warnings: Vec<String>,
    /// What changed since the previous analysis, `None` for the first one.
    diff: Option<Diff>,
}

/// What one analysis found transitively required.
///
/// By id rather than index, because we want to be able to compare this after re-ingestion.
///
/// TODO: If we want to be clever and save memory (e.g. because we assume the diff will always
/// be small?), we could instead do some hash-stuff shenanigans. Something like a hash set, but
/// without storing the underlying data unless there is a hash conflict. That sounds rough
/// though. Also makes resizing the hash set very costly, though I suppose we would know the
/// size upfront.
#[derive(Debug)]
struct Snapshot {
    /// Ids of the transitively required items.
    items: BTreeSet<String>,
    /// Ids of the recipes that propagate.
    recipes: BTreeSet<String>,
}

/// The difference between two [`Snapshot`]s.
///
/// Every list is sorted the same way [`Snapshot`] is sorted, i.e. the way strings are totally
/// ordered in Rust.
#[derive(Debug, Serialize)]
struct Diff {
    /// Items that are transitively required now, but weren't before.
    items_added: Vec<String>,
    /// Items that were transitively required before, but aren't now.
    items_removed: Vec<String>,
    /// Recipes that propagate now, but didn't before.
    recipes_added: Vec<String>,
    /// Recipes that propagated before, but don't now.
    recipes_removed: Vec<String>,
}

impl Diff {
    /// What changed from `before` to `now`.
    fn between(before: &Snapshot, now: &Snapshot) -> Self {
        /// The ids in `now` but not in `before`. Maintains the order.
        fn added(before: &BTreeSet<String>, now: &BTreeSet<String>) -> Vec<String> {
            now.difference(before).cloned().collect()
        }
        Self {
            items_added: added(&before.items, &now.items),
            items_removed: added(&now.items, &before.items),
            recipes_added: added(&before.recipes, &now.recipes),
            recipes_removed: added(&now.recipes, &before.recipes),
        }
    }
}

/// A reference to an item or recipe.
///
/// The page will use this to link to.
/// TODO: Should we not specify whether this is an item or a recipe?
#[derive(Debug, Serialize)]
struct Link {
    /// The item's or recipe's id.
    id: String,
    /// Whether it's transitively required (in case of an item) or propagates (in case of a
    /// recipe). `None` before the first analysis.
    live: Option<bool>,
}

/// The first few of a list of links, and how long the whole list is.
///
/// Mirrored by `Links` in `web/src/session-protocol.ts`.
#[derive(Debug, Serialize)]
struct Links {
    /// How many there are in total.
    total: usize,
    /// The first of them, in order, at most as many as asked for.
    ///
    /// TODO: We should be able to save a lot of memory if, instead of strings (in Link)
    /// we saved indices. Or perhaps, for each item / recipe just save its next neighbor.
    shown: Vec<Link>,
}

/// The latest successful analysis.
///
/// We keep this for the explorer.
/// TODO: Should we merge this with the state that, for sake of diffing, we already
/// keep anyway?
///
/// Only valid for the graph it ran on, so a re-ingest drops it.
#[derive(Debug)]
struct Analyzed {
    /// The rules the analysis ran with, as the one-element list [`RuleSet::apply`]'s `file`
    /// indexes.
    rule_sets: Vec<RuleSet>,
    /// Which items the rules required directly.
    attrs: NodeAttrs,
    /// Which edges the rules changed.
    edges: EdgeStates,
    /// What ended up transitively required.
    live: TransitivelyRequired,
}

/// One uploaded export, consisting of its files and the graph ingested from them.
///
/// The files stay in WASM so a changed parsing.toml can re-ingest without another upload.
#[derive(Debug)]
#[wasm_bindgen]
pub struct Session {
    /// The wrapper directory the export sat in, empty if there was no wrapper.
    root_directory: String,
    /// The export's files, paths relative to its root.
    files: Vec<RawFile>,
    /// The graph from the latest successful ingest.
    graph: Graph,
    /// The parse report from the latest successful ingest.
    report: ParseReport,
    /// Lookups into `graph`. For the explorer.
    lookups: Lookups,
    /// What the latest successful analysis found. Used to diff against the next.
    previous: Option<Snapshot>,
    /// The latest successful analysis on the current graph.
    analyzed: Option<Analyzed>,
}

#[wasm_bindgen]
#[expect(
    clippy::missing_const_for_fn,
    reason = "wasm-bindgen exported functions can't be const"
)]
impl Session {
    /// Unpacks `bytes` (a `.zip` or `.tar.zst`), finds the export inside, and ingests it with
    /// `shapes_toml` (parsing.toml's contents).
    ///
    /// # Errors
    ///
    /// If `bytes` isn't a recognized or intact archive, doesn't contain an export, or
    /// `shapes_toml` doesn't parse.
    pub fn open(bytes: &[u8], shapes_toml: &str) -> Result<Self, JsValue> {
        let unpacked = archive::unpack(bytes).map_err(|error| js_error(&error))?;
        let (root_directory, files) =
            find_root_directory::strip(unpacked).map_err(|error| js_error(&error))?;
        let (graph, report) =
            ingest::ingest(&files, shapes_toml).map_err(|error| js_error(&error))?;
        let lookups = Lookups::build(&graph);
        Ok(Self {
            root_directory,
            files,
            lookups,
            graph,
            report,
            previous: None,
            analyzed: None,
        })
    }

    /// Ingests the same files again with a new `shapes_toml`.
    ///
    /// # Errors
    ///
    /// If `shapes_toml` doesn't parse. The previous graph and report are kept then.
    ///
    /// Drops the latest analysis, which doesn't apply to the new graph.
    /// The snapshot to diff against stays, because it's by id.
    pub fn reingest(&mut self, shapes_toml: &str) -> Result<(), JsValue> {
        let (graph, report) =
            ingest::ingest(&self.files, shapes_toml).map_err(|error| js_error(&error))?;
        self.lookups = Lookups::build(&graph);
        self.graph = graph;
        self.report = report;
        self.analyzed = None;
        Ok(())
    }

    /// The wrapper directory the export sat in, empty if there was none.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn root_directory(&self) -> String {
        self.root_directory.clone()
    }

    /// How many files the export has.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// How many items and fluids the graph has.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.graph.items().len()
    }

    /// How many recipes the graph has.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn recipe_count(&self) -> usize {
        self.graph.recipes().len()
    }

    /// The parse report as JSON (mirrored by `ParseReport` in `web/src/session.ts`).
    ///
    /// # Errors
    ///
    /// Never in practice: the report is plain strings and numbers.
    pub fn report_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.report).map_err(|error| js_error(&error))
    }

    /// Applies `rules_toml` to the current graph and works out what's transitively required.
    ///
    /// Diffs the result against the previous successful analysis.
    ///
    /// # Errors
    ///
    /// If `rules_toml` isn't a valid rules file.
    pub fn analyze_json(&mut self, rules_toml: &str) -> Result<String, JsValue> {
        let rules = RuleSet::parse(rules_toml).map_err(|error| js_error(&error))?;
        let mut attrs = NodeAttrs::defaults(&self.graph);
        let mut edges = EdgeStates::defaults(&self.graph);
        rules.apply(&self.graph, &mut attrs, &mut edges, 0);
        let live = transitively_required(&self.graph, &attrs, &edges);
        let snapshot = Snapshot {
            items: live
                .transitively_required_items()
                .map(|item| self.graph.item(item).id().to_owned())
                .collect(),
            recipes: live
                .transitively_required_recipes()
                .map(|recipe| self.graph.recipe(recipe).id().to_owned())
                .collect(),
        };
        let analysis = Analysis {
            required_count: attrs.required_items().count(),
            transitively_required_item_count: live.transitively_required_item_count(),
            transitively_required_recipe_count: live.transitively_required_recipe_count(),
            warnings: rules.warnings().to_vec(),
            diff: self
                .previous
                .as_ref()
                .map(|previous| Diff::between(previous, &snapshot)),
        };
        self.previous = Some(snapshot);
        self.analyzed = Some(Analyzed {
            rule_sets: vec![rules],
            attrs,
            edges,
            live,
        });
        serde_json::to_string(&analysis).map_err(|error| js_error(&error))
    }
}
