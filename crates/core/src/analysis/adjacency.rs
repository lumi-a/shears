//! Lookups derived from a `Graph`. Not serialized, rebuilt after loading instead.

use std::collections::BTreeMap;

use crate::model::{Graph, ItemIndex, RecipeIndex};

/// Reverse index from each item to the recipes that produce it.
#[derive(Debug)]
pub struct Producers {
    /// `producers[item.index()]` lists every recipe that produces that item,
    /// paired with which of that recipe's outputs does it. Each element
    /// `(recipe, output)` of that list means `item` is produced by `recipe`,
    /// specifically by the output at index `output` in `recipe.outputs()`.
    ///
    /// `output` is `u8`, not `usize`, a recipe never has more than a handful
    /// of outputs, and we save memory that way.
    producers: Vec<Vec<(RecipeIndex, u8)>>,
}

impl Producers {
    /// Builds the reverse index from every recipe's outputs.
    ///
    /// # Panics
    ///
    /// If `graph` has more than `u32::MAX` recipes, a recipe has more than
    /// `u8::MAX` outputs, or an output references an item index out of
    /// bounds for `graph.items()`.
    #[inline]
    #[must_use]
    pub fn build(graph: &Graph) -> Self {
        let mut producers = vec![Vec::new(); graph.items().len()];
        for (recipe_position, recipe) in graph.recipes().iter().enumerate() {
            let recipe_index = RecipeIndex::new(
                u32::try_from(recipe_position).expect("fewer than u32::MAX recipes"),
            );
            for (output_position, output) in recipe.outputs().iter().enumerate() {
                let output_index = u8::try_from(output_position)
                    .expect("fewer than u8::MAX outputs per recipe");
                producers
                    .get_mut(output.item().index())
                    .expect("output item index in bounds for this graph's items")
                    .push((recipe_index, output_index));
            }
        }
        Self { producers }
    }

    /// Every `(recipe, output)` pair where `item` is that
    /// recipe's output at that position (output is an index).
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was built from.
    #[inline]
    #[must_use]
    pub fn of(&self, item: ItemIndex) -> &[(RecipeIndex, u8)] {
        self.producers
            .get(item.index())
            .expect("item index in bounds for this graph")
    }
}

/// Reverse index from each item to the recipes that take it as an input.
#[derive(Debug)]
pub struct Consumers {
    /// `consumers[item.index()]` lists every recipe that has `item` in at least one input
    /// slot. The recipes are in index order, each one once. So each list is effectively a
    /// sorted set, just cheaper than a `BTreeSet` per item, and [`Consumers::of`] can
    /// hand out a slice.
    consumers: Vec<Vec<RecipeIndex>>,
}

impl Consumers {
    /// Builds the lookup from every item to the recipes that use it.
    ///
    /// # Panics
    ///
    /// If a slot member references an item index out of bounds for `graph.items()`.
    #[inline]
    #[must_use]
    pub fn build(graph: &Graph) -> Self {
        let mut consumers: Vec<Vec<RecipeIndex>> = vec![Vec::new(); graph.items().len()];
        for recipe in graph.recipe_indices() {
            for slot in graph.recipe(recipe).inputs() {
                for member in slot.members() {
                    let list = consumers
                        .get_mut(member.index())
                        .expect("slot member index in bounds for this graph's items");
                    // Recipes come in index order, so if we have a repeat, then it can only
                    // be on the last element of the list.
                    if list.last() != Some(&recipe) {
                        list.push(recipe);
                    }
                }
            }
        }
        Self { consumers }
    }

    /// Every recipe that takes `item` as an input.
    ///
    /// Returned recipe indices are sorted by index.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was built from.
    #[inline]
    #[must_use]
    pub fn of(&self, item: ItemIndex) -> &[RecipeIndex] {
        self.consumers
            .get(item.index())
            .expect("item index in bounds for this graph")
    }
}

/// Reverse lookup from an item's id to its index.
///
/// Items and recipes share no namespace of their own, so unlike selectors (which can match
/// either), this is split into two types (see [`RecipesByKey`]).
#[derive(Debug)]
pub struct ItemsByKey {
    /// Every item's id mapped to its index.
    by_id: BTreeMap<String, ItemIndex>,
}

impl ItemsByKey {
    /// Builds the reverse lookup from every item's id.
    ///
    /// # Panics
    ///
    /// If `graph` has more than `u32::MAX` items.
    #[inline]
    #[must_use]
    pub fn build(graph: &Graph) -> Self {
        let by_id = graph
            .items()
            .iter()
            .enumerate()
            .map(|(position, item)| {
                let index = ItemIndex::new(
                    u32::try_from(position).expect("fewer than u32::MAX items"),
                );
                (item.id().to_owned(), index)
            })
            .collect();
        Self { by_id }
    }

    /// The index of the item with this id, if any.
    #[inline]
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<ItemIndex> {
        self.by_id.get(id).copied()
    }
}

/// Reverse lookup from a recipe's id to its index.
#[derive(Debug)]
pub struct RecipesByKey {
    /// Every recipe's id mapped to its index.
    by_id: BTreeMap<String, RecipeIndex>,
}

impl RecipesByKey {
    /// Builds the reverse lookup from every recipe's id.
    ///
    /// # Panics
    ///
    /// If `graph` has more than `u32::MAX` recipes.
    #[inline]
    #[must_use]
    pub fn build(graph: &Graph) -> Self {
        let by_id = graph
            .recipes()
            .iter()
            .enumerate()
            .map(|(position, recipe)| {
                let index = RecipeIndex::new(
                    u32::try_from(position).expect("fewer than u32::MAX recipes"),
                );
                (recipe.id().to_owned(), index)
            })
            .collect();
        Self { by_id }
    }

    /// The index of the recipe with this id, if any.
    #[inline]
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<RecipeIndex> {
        self.by_id.get(id).copied()
    }
}

/// Every lookup above, built together for one graph.
///
/// The graph doesn't change after ingest, so building these once and keeping them is enough.
#[derive(Debug)]
pub struct Lookups {
    /// Item id to index.
    items: ItemsByKey,
    /// Recipe id to index.
    recipes: RecipesByKey,
    /// Which recipes make each item.
    producers: Producers,
    /// Which recipes take each item as an input.
    consumers: Consumers,
}

impl Lookups {
    /// Builds every lookup for `graph`.
    ///
    /// # Panics
    ///
    /// Whenever one of [`ItemsByKey::build`], [`RecipesByKey::build`], [`Producers::build`] or
    /// [`Consumers::build`] would.
    #[inline]
    #[must_use]
    pub fn build(graph: &Graph) -> Self {
        Self {
            items: ItemsByKey::build(graph),
            recipes: RecipesByKey::build(graph),
            producers: Producers::build(graph),
            consumers: Consumers::build(graph),
        }
    }

    /// Item id to index.
    #[inline]
    #[must_use]
    pub const fn items(&self) -> &ItemsByKey {
        &self.items
    }

    /// Recipe id to index.
    #[inline]
    #[must_use]
    pub const fn recipes(&self) -> &RecipesByKey {
        &self.recipes
    }

    /// Which recipes make each item.
    #[inline]
    #[must_use]
    pub const fn producers(&self) -> &Producers {
        &self.producers
    }

    /// Which recipes take each item as an input.
    #[inline]
    #[must_use]
    pub const fn consumers(&self) -> &Consumers {
        &self.consumers
    }
}

#[cfg(test)]
mod tests {
    use super::{Consumers, ItemsByKey, Producers, RecipesByKey};
    use crate::graph_dsl::{item, parse, recipe};

    #[test]
    fn finds_every_recipe_that_takes_an_item_as_input_once() {
        let (graph, _required) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            iron_nugget: iron_ingot -> 9 iron_nugget
            iron_block: 5 iron_ingot + 4 iron_ingot -> iron_block
            ",
        );
        let consumers = Consumers::build(&graph);

        assert_eq!(consumers.of(item(&graph, "iron_pickaxe")), &[]);
        assert_eq!(
            consumers.of(item(&graph, "stick")),
            &[recipe(&graph, "iron_pickaxe")]
        );
        // In index order, which is id order, and iron_block only once.
        assert_eq!(
            consumers.of(item(&graph, "iron_ingot")),
            &[
                recipe(&graph, "iron_block"),
                recipe(&graph, "iron_nugget"),
                recipe(&graph, "iron_pickaxe"),
            ]
        );
    }

    #[test]
    fn finds_the_recipe_and_output_position_that_produce_an_item() {
        let (graph, _required) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let producers = Producers::build(&graph);

        assert_eq!(
            producers.of(item(&graph, "iron_pickaxe")),
            &[(recipe(&graph, "iron_pickaxe"), 0)]
        );
        assert_eq!(producers.of(item(&graph, "iron_ingot")), &[]);
        assert_eq!(producers.of(item(&graph, "stick")), &[]);
    }

    #[test]
    fn aggregates_producers_across_different_recipes() {
        let (graph, _required) = parse(
            "
            iron_ingot_from_nuggets: 9 iron_nugget -> iron_ingot
            iron_ingot_from_smelting_raw_iron: raw_iron -> iron_ingot
            ",
        );
        let producers = Producers::build(&graph);

        assert_eq!(
            producers.of(item(&graph, "iron_ingot")),
            &[
                (recipe(&graph, "iron_ingot_from_nuggets"), 0),
                (recipe(&graph, "iron_ingot_from_smelting_raw_iron"), 0),
            ]
        );
    }

    #[test]
    fn keeps_both_entries_when_one_recipe_outputs_the_same_item_twice() {
        let (graph, _required) = parse(
            "
            gtceu:arc_furnace/arc_max_electric_pump: monilabs:max_electric_pump -> 64 gtceu:monium_ingot + 12 gtceu:meta_null_ingot + 4 gtceu:monium_ingot + 2 gtceu:terbium_ingot
            ",
        );
        let producers = Producers::build(&graph);

        let recycling = recipe(&graph, "gtceu:arc_furnace/arc_max_electric_pump");
        assert_eq!(
            producers.of(item(&graph, "gtceu:monium_ingot")),
            &[(recycling, 0), (recycling, 2)]
        );
    }

    #[test]
    fn items_by_key_finds_the_index_of_a_known_id() {
        let (graph, _required) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let by_key = ItemsByKey::build(&graph);

        for id in ["iron_pickaxe", "iron_ingot", "stick"] {
            assert_eq!(by_key.index_of(id), Some(item(&graph, id)), "{id}");
        }
    }

    #[test]
    fn items_by_key_returns_none_for_an_unknown_id() {
        let (graph, _required) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let by_key = ItemsByKey::build(&graph);

        assert_eq!(by_key.index_of("dirt"), None);
    }

    #[test]
    fn recipes_by_key_finds_the_index_of_a_known_id() {
        let (graph, _required) = parse(
            "
            iron_ingot_from_nuggets: 9 iron_nugget -> iron_ingot
            iron_ingot_from_smelting_raw_iron: raw_iron -> iron_ingot
            ",
        );
        let by_key = RecipesByKey::build(&graph);

        for id in [
            "iron_ingot_from_nuggets",
            "iron_ingot_from_smelting_raw_iron",
        ] {
            assert_eq!(by_key.index_of(id), Some(recipe(&graph, id)), "{id}");
        }
    }

    #[test]
    fn recipes_by_key_returns_none_for_an_unknown_id() {
        let (graph, _required) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let by_key = RecipesByKey::build(&graph);

        assert_eq!(by_key.index_of("nonexistent"), None);
    }
}
