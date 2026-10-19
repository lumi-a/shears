//! Items, recipes and the graph that holds them.
//!
//! These are plain data: construction, adjacency and analysis live elsewhere.
//!
//! Terminology:
//! - An **id**, as in `Item::id` or `Recipe::id`, is the namespaced string
//! Minecraft itself calls an id (or "Identifier"), e.g. `"minecraft:iron_ingot"`.
//! - An **index**, as in `ItemIndex` or `RecipeIndex`, is this crate's own `u32`
//! position into `Graph::items` or `Graph::recipes`. It has no Minecraft meaning
//! and is only ever valid for the one `Graph` it came from.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Index into `Graph::items`.
///
/// Only meaningful for the specific [`Graph`] that produced it.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ItemIndex(u32);

impl ItemIndex {
    /// Constructs the index for the item at this position in [`Graph::items`].
    ///
    /// The caller is responsible for the index being coupled to the correct [`Graph`].
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// This index's position as `usize`.
    ///
    /// Useful for indexing into a side table shaped like [`Graph::items`].
    ///
    /// # Panics
    ///
    /// Never panics on the 32- and 64-bit targets this crate supports.
    /// `u32` always fits in `usize` there.
    #[inline]
    #[must_use]
    pub fn index(self) -> usize {
        usize::try_from(self.0).expect("u32 always fits in usize on supported targets")
    }
}

/// Index into `Graph::recipes`.
///
/// Only meaningful for the specific [`Graph`] that produced it.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct RecipeIndex(u32);

impl RecipeIndex {
    /// Constructs the index for the recipe at this position in [`Graph::recipes`].
    ///
    /// The caller is responsible for the index being tied to the correct [`Graph`].
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// This index's position as `usize`.
    ///
    /// Useful for indexing into a side table shaped like [`Graph::items`].
    ///
    /// # Panics
    ///
    /// Never panics on the 32- and 64-bit targets this crate supports.
    /// `u32` always fits in `usize` there.
    #[inline]
    #[must_use]
    pub fn index(self) -> usize {
        usize::try_from(self.0).expect("u32 always fits in usize on supported targets")
    }
}

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

impl Item {
    /// Constructs an item or fluid.
    pub(crate) const fn new(id: String, kind: ItemKind, tags: BTreeSet<u32>) -> Self {
        Self { id, kind, tags }
    }

    /// The full id, e.g. `minecraft:iron_ingot`.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether this is an item or fluid.
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> ItemKind {
        self.kind
    }

    /// Indices into [`Graph::tags`] for every tag of this [`Item`].
    #[inline]
    #[must_use]
    pub const fn tags(&self) -> &BTreeSet<u32> {
        &self.tags
    }
}

/// One input slot of a [`Recipe`]. This is an OR over the slot's members.
#[derive(Debug, Serialize, Deserialize)]
pub struct Slot {
    /// Tag-expanded alternatives for this slot.
    members: BTreeSet<ItemIndex>,
    /// Index into [`Graph::tags`] for the tag this slot was expanded from, if any.
    tag: Option<u32>,
    /// Number of items or millibuckets.
    count: u32,
    /// Chance that this slot is consumed, in `[0, 1]`. Catalyst are chance 0.
    ///
    /// `GTCEu` encodes this as `chance / maxChance`, with
    /// maxChance == 10000 if unspecified.
    chance: f32,
}

impl Slot {
    /// Constructs an input slot.
    ///
    /// TODO: Should we check that `chance` is in [0, 1]?
    pub(crate) const fn new(
        members: BTreeSet<ItemIndex>,
        tag: Option<u32>,
        count: u32,
        chance: f32,
    ) -> Self {
        Self {
            members,
            tag,
            count,
            chance,
        }
    }

    /// Tag-expanded alternatives for this slot.
    #[inline]
    #[must_use]
    pub const fn members(&self) -> &BTreeSet<ItemIndex> {
        &self.members
    }

    /// Index into [`Graph::tags`] for the tag this slot was expanded from, if any.
    #[inline]
    #[must_use]
    pub const fn tag(&self) -> Option<u32> {
        self.tag
    }

    /// Number of items or millibuckets.
    #[inline]
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// Chance that this slot is consumed, in `[0, 1]`.
    #[inline]
    #[must_use]
    pub const fn chance(&self) -> f32 {
        self.chance
    }
}

/// One output of a [`Recipe`].
#[derive(Debug, Serialize, Deserialize)]
pub struct Output {
    /// The item or fluid produced.
    item: ItemIndex,
    /// How many items or how many millibuckets are produced.
    count: u32,
    /// Chance of this output occurring, in `[0, 1]`.
    ///
    /// `GTCEu` encodes this as `chance / maxChance`, with
    /// maxChance == 10000 if unspecified.
    chance: f32,
}

impl Output {
    /// Constructs a recipe output.
    ///
    /// TODO: Should we check that `chance` is in \[0,1\] or even (0,1\]?
    pub(crate) const fn new(item: ItemIndex, count: u32, chance: f32) -> Self {
        Self {
            item,
            count,
            chance,
        }
    }

    /// The item or fluid produced.
    #[inline]
    #[must_use]
    pub const fn item(&self) -> ItemIndex {
        self.item
    }

    /// How many items or millibuckets are produced.
    #[inline]
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// Chance of this output occurring, in `[0, 1]`.
    #[inline]
    #[must_use]
    pub const fn chance(&self) -> f32 {
        self.chance
    }
}

/// A recipe: a node on the "process" side of the bipartite graph.
#[derive(Debug, Serialize, Deserialize)]
pub struct Recipe {
    /// The recipe id, e.g. `minecraft:iron_ingot_from_smelting`.
    id: String,
    /// Index into [`Graph::recipe_types`] for this recipe's type,
    /// e.g. `minecraft:smelting`.
    rtype: u32,
    /// Inputs. AND over slots, an OR within each slots options (usually tag-expanded).
    inputs: Vec<Slot>,
    /// Outputs of this recipe.
    outputs: Vec<Output>,
    /// EU/t, for recipes that have it.
    eu_t: Option<i64>,
    /// Duration in ticks, for recipes that have it.
    ticks: Option<u32>,
}

impl Recipe {
    /// Constructs a recipe.
    pub(crate) const fn new(
        id: String,
        rtype: u32,
        inputs: Vec<Slot>,
        outputs: Vec<Output>,
        eu_t: Option<i64>,
        ticks: Option<u32>,
    ) -> Self {
        Self {
            id,
            rtype,
            inputs,
            outputs,
            eu_t,
            ticks,
        }
    }

    /// The recipe id, e.g. `minecraft:iron_ingot_from_smelting`.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Index into [`Graph::recipe_types`] for this recipe's type.
    #[inline]
    #[must_use]
    pub const fn rtype(&self) -> u32 {
        self.rtype
    }

    /// Inputs. AND over slots, an OR within each slot's members.
    #[inline]
    #[must_use]
    pub fn inputs(&self) -> &[Slot] {
        &self.inputs
    }

    /// Outputs of this recipe.
    #[inline]
    #[must_use]
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// EU/t, for recipes that have it.
    #[inline]
    #[must_use]
    pub const fn eu_t(&self) -> Option<i64> {
        self.eu_t
    }

    /// Duration in ticks, for recipes that have it.
    #[inline]
    #[must_use]
    pub const fn ticks(&self) -> Option<u32> {
        self.ticks
    }
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
    /// Interned recipe type names. These are indexed by [`Recipe::rtype`].
    recipe_types: Vec<String>,
}

impl Graph {
    /// Assembles a graph from already-sorted, already-interned parts.
    ///
    /// The caller is responsible for `items` and `recipes` being sorted by id, and for
    /// every index referenced from within them being valid for the `tags` and
    /// `recipe_types`.
    pub(crate) const fn new(
        schema: u32,
        items: Vec<Item>,
        recipes: Vec<Recipe>,
        tags: Vec<String>,
        recipe_types: Vec<String>,
    ) -> Self {
        Self {
            schema,
            items,
            recipes,
            tags,
            recipe_types,
        }
    }

    /// Schema version of this file, for sake of forward compatibility.
    #[inline]
    #[must_use]
    pub const fn schema(&self) -> u32 {
        self.schema
    }

    /// Items and fluids, sorted by [`Item::id`].
    #[inline]
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Recipes, sorted by [`Recipe::id`].
    #[inline]
    #[must_use]
    pub fn recipes(&self) -> &[Recipe] {
        &self.recipes
    }

    /// Interned tag names. These are indexed by [`Item::tags`] and [`Slot::tag`].
    #[inline]
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Interned recipe type names. These are indexed by [`Recipe::rtype`].
    #[inline]
    #[must_use]
    pub fn recipe_types(&self) -> &[String] {
        &self.recipe_types
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, Slot};

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
                rtype: 0,
                inputs: vec![Slot {
                    members: BTreeSet::from([ItemIndex(1)]),
                    tag: None,
                    count: 1,
                    chance: 1.0,
                }],
                outputs: vec![Output {
                    item: ItemIndex(0),
                    count: 1,
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

    proptest::proptest! {
        #[test]
        fn item_index_round_trips_through_usize(raw: u32) {
            let expected = usize::try_from(raw).expect("u32 always fits in usize on supported targets");
            proptest::prop_assert_eq!(ItemIndex::new(raw).index(), expected);
        }

        #[test]
        fn recipe_index_round_trips_through_usize(raw: u32) {
            let expected = usize::try_from(raw).expect("u32 always fits in usize on supported targets");
            proptest::prop_assert_eq!(super::RecipeIndex::new(raw).index(), expected);
        }
    }
}
