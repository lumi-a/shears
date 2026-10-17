//! Ids, items, recipes and the graph that holds them.
//!
//! These are plain data: construction, adjacency and analysis live elsewhere.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Index into [`Graph::items`].
///
/// Only meaningful for the specific [`Graph`] that produced it.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ItemId(u32);

/// Index into [`Graph::recipes`].
///
/// Only meaningful for the specific [`Graph`] that produced it.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct RecipeId(u32);

/// What kind of stack an [`Item`] is.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ItemKind {
    /// A regular item stack.
    Item,
    /// A fluid, in millibuckets.
    Fluid,
}

/// An item or fluid.
#[derive(Debug, Serialize, Deserialize)]
pub struct Item {
    /// The full id, e.g. `minecraft:iron_ingot`.
    id: String,
    /// Whether this is an item or fluid.
    kind: ItemKind,
    /// Indices into [`Graph::tags`] for every tag of this [`Item`].
    tags: BTreeSet<u32>,
}

/// One input slot of a [`Recipe`]. This is an OR over the slot's members.
#[derive(Debug, Serialize, Deserialize)]
pub struct Slot {
    /// Tag-expanded alternatives for this slot.
    members: BTreeSet<ItemId>,
    /// Index into [`Graph::tags`] for the tag this slot was expanded from, if any.
    tag: Option<u32>,
    /// Number of items or millibuckets.
    count: f32,
    /// Chance that this slot is consumed, in `[0, 1]`. Catalyst are chance 0.
    ///
    /// GTCEu encodes this as `chance / maxChance`, with
    /// maxChance == 10000 if unspecified.
    chance: f32,
}

/// One output of a [`Recipe`].
#[derive(Debug, Serialize, Deserialize)]
pub struct Output {
    /// The item or fluid produced.
    item: ItemId,
    /// How many items or how many millibuckets are produced.
    count: f32,
    /// Chance of this output occurring, in `[0, 1]`.
    ///
    /// GTCEu encodes this as `chance / maxChance`, with
    /// maxChance == 10000 if unspecified.
    chance: f32,
}

/// A recipe: a node on the "process" side of the bipartite graph.
#[derive(Debug, Serialize, Deserialize)]
pub struct Recipe {
    /// The recipe id, e.g. `minecraft:iron_ingot_from_smelting`.
    id: String,
    /// Index into [`Graph::recipe_types`] for this recipe's type,
    /// e.g. `minecraft:smelting`.
    recipe_type: u32,
    /// Inputs. AND over slots, an OR within each slots options (usually tag-expanded).
    inputs: Vec<Slot>,
    /// Outputs of this recipe.
    outputs: Vec<Output>,
    /// EU/t, for recipes that have it.
    eu_t: Option<i64>,
    /// Duration in ticks, for recipes that have it.
    ticks: Option<u32>,
}

/// The bipartite graph of items and recipes.
///
/// Items and recipes are sorted by key to keep serialized output    deterministic.
///
/// Returned by ingest, and used by analysis.
#[derive(Debug, Serialize, Deserialize)]
pub struct Graph {
    /// Schema version of this file, for sake of forward compatibility.
    schema: u32,
    /// Items and fluids, sorted by [`Item::id`].
    items: Vec<Item>,
    /// Recipes, sorted by [`Recipe::id`].
    recipes: Vec<Recipe>,
    /// Interned tag names. These are indexed by [`Item::tags`] and [`Slot::tag`].
    tags: Vec<String>,
    /// Interned recipe type names. These are indexed by [`Recipe::recipe_type`].
    recipe_types: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Graph, Item, ItemId, ItemKind, Output, Recipe, Slot};

    #[test]
    fn graph_json_round_trip_is_stable() {
        let graph = Graph {
            schema: 1,
            items: vec![
                Item {
                    id: "minecraft:iron_ingot".to_owned(),
                    kind: ItemKind::Item,
                    tags: BTreeSet::from([0]),
                },
                Item {
                    id: "minecraft:iron_ore".to_owned(),
                    kind: ItemKind::Item,
                    tags: BTreeSet::new(),
                },
            ],
            recipes: vec![Recipe {
                id: "minecraft:iron_ingot_from_smelting".to_owned(),
                recipe_type: 0,
                inputs: vec![Slot {
                    members: BTreeSet::from([ItemId(1)]),
                    tag: None,
                    count: 1.0,
                    chance: 1.0,
                }],
                outputs: vec![Output {
                    item: ItemId(0),
                    count: 1.0,
                    chance: 1.0,
                }],
                eu_t: None,
                ticks: Some(200),
            }],
            tags: vec!["forge:ingots/iron".to_owned()],
            recipe_types: vec!["minecraft:smelting".to_owned()],
        };

        let first = serde_json::to_string(&graph).expect("serializing a new Graph");
        let deserialized: Graph = serde_json::from_str(&first)
            .expect("deserializing shears' own graph.json serde_json output");
        let second =
            serde_json::to_string(&deserialized).expect("serializing the deserialized Graph again");

        assert_eq!(first, second, "graph.json should round-trip identically");
    }
}
