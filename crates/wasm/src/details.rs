//! The explorer's detail view showing info about one item or recipe.

use std::collections::BTreeSet;

use serde::Serialize;
use shears_core::analysis::edges::EdgeState;
use shears_core::analysis::explain::cause_text;
use shears_core::analysis::transitively_required::Why;
use shears_core::ingest;
use shears_core::model::{Graph, ItemIndex, ItemKind, RecipeIndex};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{Analyzed, Kind, Link, Links, Session, js_error};

/// One step of why an item is transitively required.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
enum WhyStep {
    /// The item before this step is an input of `recipe`, which makes the transitively
    /// required `output`. The next step explains `output`.
    Via {
        /// The recipe.
        recipe: Link,
        /// The output.
        output: Link,
    },
    /// The item before this step is required directly, for this reason. Always the last step.
    Required(String),
}

/// Everything about one item.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
struct ItemDetail {
    /// Its id.
    id: String,
    /// Whether it's an item or a fluid.
    fluid: bool,
    /// Every tag it has, sorted.
    tags: Vec<String>,
    /// Why it's required directly, if a rule requires it.
    ///
    /// `None` before the first analysis, or if it's not required.
    required: Option<String>,
    /// Whether it's transitively required.
    ///
    /// `None` before the first analysis, or if it's not transitively required.
    live: Option<bool>,
    /// Why it's transitively required, from this item to a required one.
    ///
    /// Empty before the first analysis, or if it's not transitively required.
    why: Vec<WhyStep>,
    /// The recipes that make it.
    produced_by: Links,
    /// The recipes that take it as an input.
    used_in: Links,
}

/// One input slot of a recipe.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
struct InputDetail {
    /// The tag this slot was expanded from, if any.
    tag: Option<String>,
    /// How many items, or (in case of a fluid) millibuckets.
    count: u32,
    /// The chance this slot is consumed. Zero for a catalyst.
    chance: f32,
    /// The items that can go in this slot.
    members: Links,
}

/// One output of a recipe.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
struct OutputDetail {
    /// The item made.
    item: Link,
    /// How many items, or (in case of a fluid) millibuckets.
    count: u32,
    /// The chance that this output is produced.
    chance: f32,
    /// `"propagate"`, `"inert"` or `"dropped"`.
    ///
    /// `None` before the first analysis.
    #[cfg_attr(test, ts(type = "\"propagate\" | \"inert\" | \"dropped\" | null"))]
    edge: Option<&'static str>,
    /// Why the edge is in that state.
    ///
    /// `None` before the first analysis.
    edge_cause: Option<String>,
}

/// Everything about one recipe.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
struct RecipeDetail {
    /// Its id.
    id: String,
    /// The path of the file it was read from, for [`Session::file_text`].
    file: Option<String>,
    /// Its recipe type, e.g. `minecraft:crafting_shaped`.
    recipe_type: String,
    /// EU/t, for recipes that have it.
    // A JSON number, which ts-rs would type as a bigint.
    #[cfg_attr(test, ts(type = "number | null"))]
    eu_per_tick: Option<i64>,
    /// Duration in ticks, for recipes that have it.
    ticks: Option<u32>,
    /// Whether it propagates. `None` before the first analysis.
    live: Option<bool>,
    /// Its input slots.
    inputs: Vec<InputDetail>,
    /// Its outputs.
    outputs: Vec<OutputDetail>,
}

/// The name of `state`.
const fn edge_name(state: EdgeState) -> &'static str {
    match state {
        EdgeState::Propagate => "propagate",
        EdgeState::Inert => "inert",
        EdgeState::Dropped => "dropped",
    }
}

/// What [`Session`] needs when turning indices into links.
struct Linker<'session> {
    /// The current graph.
    graph: &'session Graph,
    /// The latest analysis, if any.
    analyzed: Option<&'session Analyzed>,
    /// The most links any one list gets.
    limit: usize,
}

impl Linker<'_> {
    /// A link to `item`.
    fn item(&self, item: ItemIndex) -> Link {
        Link {
            kind: Kind::Item,
            id: self.graph.item(item).id().to_owned(),
            live: self
                .analyzed
                .map(|analyzed| analyzed.live.is_transitively_required(item)),
        }
    }

    /// A link to `recipe`.
    fn recipe(&self, recipe: RecipeIndex) -> Link {
        Link {
            kind: Kind::Recipe,
            id: self.graph.recipe(recipe).id().to_owned(),
            live: self
                .analyzed
                .map(|analyzed| analyzed.live.is_recipe_transitively_required(recipe)),
        }
    }

    /// Links to the first `limit` of `targets`, and how many there are.
    fn list<T>(
        &self,
        targets: impl ExactSizeIterator<Item = T>,
        link: impl Fn(&Self, T) -> Link,
    ) -> Links {
        Links {
            total: targets.len(),
            shown: targets
                .take(self.limit)
                .map(|target| link(self, target))
                .collect(),
        }
    }

    /// Why `item` is transitively required, following the analysis's chain of reasons.
    fn why(&self, item: ItemIndex) -> Vec<WhyStep> {
        let Some(analyzed) = self.analyzed else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        let mut current = item;
        // Each step moves to the item that made the current one required, which was
        // reached earlier in the analysis, so no item comes up twice.
        // But should one anyway, stop there instead of going in circles.
        let mut visited = BTreeSet::new();
        while visited.insert(current) {
            match analyzed.live.why(current) {
                None => break,
                Some(Why::Required(cause)) => {
                    steps.push(WhyStep::Required(cause_text(&analyzed.rule_sets, cause)));
                    break;
                }
                Some(&Why::Via { recipe, output }) => {
                    steps.push(WhyStep::Via {
                        recipe: self.recipe(recipe),
                        output: self.item(output),
                    });
                    current = output;
                }
            }
        }
        steps
    }
}

#[wasm_bindgen]
#[expect(
    clippy::multiple_inherent_impl,
    reason = "the explorer's part of Session's JS API. Kept apart from lib.rs's core lifecycle"
)]
impl Session {
    /// A linker for this session, with lists cut off after `limit`.
    const fn linker(&self, limit: usize) -> Linker<'_> {
        Linker {
            graph: &self.graph,
            analyzed: self.analyzed.as_ref(),
            limit,
        }
    }

    /// Everything about the item `id` as JSON (mirrored by `ItemDetail` in
    /// `web/src/session-protocol.ts`), with each list cut off after `limit`.
    ///
    /// # Errors
    ///
    /// If there's no item `id`.
    pub fn item_json(&self, id: &str, limit: usize) -> Result<String, JsValue> {
        let item = self
            .lookups
            .items()
            .index_of(id)
            .ok_or_else(|| js_error(&format!("no item {id:?}")))?;
        let linker = self.linker(limit);
        let data = self.graph.item(item);
        let detail = ItemDetail {
            id: data.id().to_owned(),
            fluid: data.kind() == ItemKind::Fluid,
            tags: data
                .tags()
                .iter()
                .filter_map(|&tag| self.graph.tags().get(usize::try_from(tag).ok()?).cloned())
                .collect(),
            required: self.analyzed.as_ref().and_then(|analyzed| {
                analyzed
                    .attrs
                    .required_cause(item)
                    .map(|cause| cause_text(&analyzed.rule_sets, cause))
            }),
            live: linker
                .analyzed
                .map(|analyzed| analyzed.live.is_transitively_required(item)),
            why: linker.why(item),
            produced_by: linker.list(
                self.lookups
                    .producers()
                    .of(item)
                    .iter()
                    .map(|&(recipe, _output)| recipe),
                Linker::recipe,
            ),
            used_in: linker.list(
                self.lookups.consumers().of(item).iter().copied(),
                Linker::recipe,
            ),
        };
        serde_json::to_string(&detail).map_err(|error| js_error(&error))
    }

    /// Everything about the recipe `id` as JSON (mirrored by `RecipeDetail` in
    /// `web/src/session-protocol.ts`), with each slot's members cut off after `limit`.
    ///
    /// # Errors
    ///
    /// If there's no recipe `id`.
    pub fn recipe_json(&self, id: &str, limit: usize) -> Result<String, JsValue> {
        let recipe = self
            .lookups
            .recipes()
            .index_of(id)
            .ok_or_else(|| js_error(&format!("no recipe {id:?}")))?;
        let linker = self.linker(limit);
        let data = self.graph.recipe(recipe);
        let name_of = |index: u32, names: &[String]| {
            usize::try_from(index)
                .ok()
                .and_then(|position| names.get(position))
                .cloned()
        };
        let detail = RecipeDetail {
            id: data.id().to_owned(),
            file: ingest::recipe_path(data.id()),
            recipe_type: name_of(data.rtype(), self.graph.recipe_types()).unwrap_or_default(),
            eu_per_tick: data.eu_t(),
            ticks: data.ticks(),
            live: linker
                .analyzed
                .map(|analyzed| analyzed.live.is_recipe_transitively_required(recipe)),
            inputs: data
                .inputs()
                .iter()
                .map(|slot| InputDetail {
                    tag: slot.tag().and_then(|tag| name_of(tag, self.graph.tags())),
                    count: slot.count(),
                    chance: slot.chance(),
                    members: linker.list(slot.members().iter().copied(), Linker::item),
                })
                .collect(),
            outputs: data
                .outputs()
                .iter()
                .zip(0_u8..)
                .map(|(output, position)| {
                    let edge = self.analyzed.as_ref().map(|analyzed| {
                        (
                            edge_name(analyzed.edges.state(recipe, position)),
                            cause_text(
                                &analyzed.rule_sets,
                                analyzed.edges.cause(recipe, position),
                            ),
                        )
                    });
                    OutputDetail {
                        item: linker.item(output.item()),
                        count: output.count(),
                        chance: output.chance(),
                        edge: edge.as_ref().map(|&(name, _)| name),
                        edge_cause: edge.map(|(_, cause)| cause),
                    }
                })
                .collect(),
        };
        serde_json::to_string(&detail).map_err(|error| js_error(&error))
    }
}
