//! The computation of which items and recipes are transitively required.
//!
//! ```txt
//! transitively_required(i) ⟺
//!   i ∈ required OR
//!   ∃ recipe r, output o:
//!     i is an input of r AND
//!     o is an output of r AND
//!     transitively_required(o) == true AND
//!     state(r, o) = propagate
//! ```
//! Starting from the required items and walking backwards through producers.
//!
//! The other alternatives, i.e. starting from non-required recipes with no transitively
//! required successors, marking them as dead, and iterating, maintains the self-sustaining
//! junk-cycles like (stone dust <-> small stone dust). Propagating transitive requirement
//! prevents this.

use core::iter::repeat_with;
use std::collections::VecDeque;

use fixedbitset::FixedBitSet;

use super::adjacency::Producers;
use super::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
use crate::model::{Graph, ItemIndex, RecipeIndex};

/// Why an item ended up transitively required.
#[derive(Debug)]
pub enum Why {
    /// Transitively required because it's required directly. Also explains why it was
    /// required.
    Required(Cause),
    /// Transitively required because `recipe`'s output `output` is transitively
    /// required, with a propagating edge.
    Via {
        /// The recipe that has this item as an input.
        recipe: RecipeIndex,
        /// The transitively required output that made this item transitively required.
        output: ItemIndex,
    },
}

/// The result of [`transitively_required`]: which items and recipes are transitively required
/// and why.
#[derive(Debug)]
pub struct TransitivelyRequired {
    /// `item_bits.contains(item.index())`: whether `item` is transitively required.
    item_bits: FixedBitSet,
    /// `recipe_bits.contains(recipe.index())`: whether `recipe` has at
    /// least one transitively required output over a propagating edge.
    recipe_bits: FixedBitSet,
    /// `why[item.index()]` is why that item is transitively required, or `None` if it's
    /// dead.
    why: Vec<Option<Why>>,
}

impl TransitivelyRequired {
    /// Whether `item` is transitively required.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was computed from.
    #[inline]
    #[must_use]
    pub fn is_transitively_required(&self, item: ItemIndex) -> bool {
        self.item_bits.contains(item.index())
    }

    /// Whether `recipe` propagates, i.e. whether at least one of its
    /// outputs is transitively required over a propagating edge.
    ///
    /// # Panics
    ///
    /// If `recipe` is out of bounds for the graph this was computed from.
    #[inline]
    #[must_use]
    pub fn is_recipe_transitively_required(&self, recipe: RecipeIndex) -> bool {
        self.recipe_bits.contains(recipe.index())
    }

    /// Why `item` is transitively required, or `None` if it's dead.
    #[inline]
    #[must_use]
    pub fn why(&self, item: ItemIndex) -> Option<&Why> {
        self.why.get(item.index()).and_then(Option::as_ref)
    }

    /// How many items are transitively required.
    #[inline]
    #[must_use]
    pub fn transitively_required_item_count(&self) -> usize {
        self.item_bits.count_ones(..)
    }

    /// How many recipes propagate, i.e. have at least one transitively required output
    /// over a propagating edge.
    #[inline]
    #[must_use]
    pub fn transitively_required_recipe_count(&self) -> usize {
        self.recipe_bits.count_ones(..)
    }

    /// Every transitively required item.
    ///
    /// Returned in index order.
    ///
    /// # Panics
    ///
    /// If the graph has more than `u32::MAX` items.
    #[inline]
    pub fn transitively_required_items(&self) -> impl Iterator<Item = ItemIndex> + '_ {
        self.item_bits.ones().map(|position| {
            ItemIndex::new(u32::try_from(position).expect("fewer than u32::MAX items"))
        })
    }

    /// Every recipe that propagates, i.e. has at least one transitively required output over a
    /// propagating edge.
    ///
    /// Returned in index order.
    ///
    /// # Panics
    ///
    /// If the graph has more than `u32::MAX` recipes.
    #[inline]
    pub fn transitively_required_recipes(&self) -> impl Iterator<Item = RecipeIndex> + '_ {
        self.recipe_bits.ones().map(|position| {
            RecipeIndex::new(u32::try_from(position).expect("fewer than u32::MAX recipes"))
        })
    }
}

/// Computes what's transitively required.
///
/// Reverse BFS from `attrs`'s required items, over `graph`'s recipes, and only following edges
/// `edges` marked [`EdgeState::Propagate`].
///
/// # Panics
///
/// If `attrs`, `edges` or their indices weren't built from `graph`.
#[inline]
#[must_use]
pub fn transitively_required(
    graph: &Graph,
    attrs: &NodeAttrs,
    edges: &EdgeStates,
) -> TransitivelyRequired {
    let producers = Producers::build(graph);

    let mut item_bits = FixedBitSet::with_capacity(graph.items().len());
    let mut recipe_bits = FixedBitSet::with_capacity(graph.recipes().len());
    let mut why: Vec<Option<Why>> = repeat_with(|| None).take(graph.items().len()).collect();
    // TODO: Once we have multiple sets to BFS over, we probably should still
    // only have one queue (marked as "is this relevant to *anything*"),
    // coupled with a bitvec relevant[i] "is this item relevant to ruleset i?".
    // Does that make sense?
    let mut queue: VecDeque<ItemIndex> = VecDeque::new();

    for required in attrs.required_items() {
        item_bits.insert(required.index());
        let cause = attrs
            .required_cause(required)
            .expect(
                "a required item always has a cause, set alongside it by NodeAttrs::set_required",
            )
            .clone();
        *why.get_mut(required.index())
            .expect("required index in bounds for this graph") = Some(Why::Required(cause));
        queue.push_back(required);
    }

    while let Some(output_item) = queue.pop_front() {
        for &(recipe, output) in producers.of(output_item) {
            if edges.state(recipe, output) != EdgeState::Propagate {
                continue;
            }
            if recipe_bits.contains(recipe.index()) {
                continue;
            }
            recipe_bits.insert(recipe.index());

            let recipe_data = graph.recipe(recipe);
            for slot in recipe_data.inputs() {
                for &member in slot.members() {
                    if item_bits.contains(member.index()) {
                        continue;
                    }
                    item_bits.insert(member.index());
                    *why.get_mut(member.index())
                        .expect("item index in bounds for this graph") = Some(Why::Via {
                        recipe,
                        output: output_item,
                    });
                    queue.push_back(member);
                }
            }
        }
    }

    TransitivelyRequired {
        item_bits,
        recipe_bits,
        why,
    }
}

/// Naive reference version of [`transitively_required`]. Iterates the fixpoint equation to a
/// fixed point by rescanning every recipe, instead of BFS.
///
/// This is only used to test [`transitively_required`] against a structurally different
/// implementation of the equation.
#[cfg(test)]
fn naive_transitively_required(
    graph: &Graph,
    attrs: &NodeAttrs,
    edges: &EdgeStates,
) -> (FixedBitSet, FixedBitSet) {
    let mut item_bits = FixedBitSet::with_capacity(graph.items().len());
    for position in 0..graph.items().len() {
        let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
        if attrs.is_required(item) {
            item_bits.insert(position);
        }
    }
    let mut recipe_bits = FixedBitSet::with_capacity(graph.recipes().len());

    loop {
        let mut changed = false;
        for (recipe_position, recipe) in graph.recipes().iter().enumerate() {
            let recipe_index = RecipeIndex::new(
                u32::try_from(recipe_position).expect("recipe count fits u32"),
            );
            for (output_position, output) in recipe.outputs().iter().enumerate() {
                let output_index =
                    u8::try_from(output_position).expect("outputs per recipe fit u8");
                if edges.state(recipe_index, output_index) != EdgeState::Propagate {
                    continue;
                }
                if !item_bits.contains(output.item().index()) {
                    continue;
                }
                if !recipe_bits.contains(recipe_position) {
                    recipe_bits.insert(recipe_position);
                    changed = true;
                }
                for slot in recipe.inputs() {
                    for &member in slot.members() {
                        if !item_bits.contains(member.index()) {
                            item_bits.insert(member.index());
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    (item_bits, recipe_bits)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Why, naive_transitively_required, transitively_required};
    use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
    use crate::analysis::proptest_support::arb_graph_attrs_edges;
    use crate::graph_dsl::{item, parse, recipe};
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn iterates_only_transitively_required_items_and_recipes() {
        let (graph, required_items) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            oak_planks: oak_log -> 4 oak_planks
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        let items: Vec<&str> = live
            .transitively_required_items()
            .map(|item| graph.item(item).id())
            .collect();
        let recipes: Vec<&str> = live
            .transitively_required_recipes()
            .map(|recipe| graph.recipe(recipe).id())
            .collect();
        assert_eq!(items, ["iron_ingot", "iron_pickaxe", "stick"]);
        assert_eq!(recipes, ["iron_pickaxe"]);
    }

    #[test]
    fn marks_required_items_and_their_producer_chain_transitively_required() {
        let (graph, required_items) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        let iron_pickaxe = item(&graph, "iron_pickaxe");
        let iron_ingot = item(&graph, "iron_ingot");
        let stick = item(&graph, "stick");
        let crafting = recipe(&graph, "iron_pickaxe");

        assert!(live.is_transitively_required(iron_pickaxe));
        assert!(live.is_transitively_required(iron_ingot));
        assert!(live.is_transitively_required(stick));
        assert!(live.is_recipe_transitively_required(crafting));

        assert!(matches!(
            live.why(iron_pickaxe),
            Some(Why::Required(Cause::Pseudo))
        ));
        assert!(matches!(
            live.why(iron_ingot),
            Some(Why::Via { recipe, output }) if *recipe == crafting && *output == iron_pickaxe
        ));
    }

    #[test]
    fn nugget_cycle_dies_entirely_when_nothing_is_required() {
        let (graph, required_items) = parse(
            "
            iron_ingot_from_nuggets: 9 iron_nugget -> iron_ingot
            iron_nugget: iron_ingot -> 9 iron_nugget
            ",
        );
        assert!(
            required_items.is_empty(),
            "no required line in this fixture"
        );
        let attrs = NodeAttrs::defaults(&graph);
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        assert!(!live.is_transitively_required(item(&graph, "iron_ingot")));
        assert!(!live.is_transitively_required(item(&graph, "iron_nugget")));
        assert!(
            !live.is_recipe_transitively_required(recipe(&graph, "iron_ingot_from_nuggets"))
        );
        assert!(!live.is_recipe_transitively_required(recipe(&graph, "iron_nugget")));
    }

    #[test]
    fn iron_ingot_used_keeps_nuggets_dead_if_ingot_from_nuggets_is_inert() {
        let (graph, required_items) = parse(
            "
            required iron_pickaxe
            iron_pickaxe:            3 iron_ingot + 2 stick -> iron_pickaxe
            iron_ingot_from_nuggets: 9 iron_nugget -> iron_ingot
            iron_nugget:             iron_ingot -> 9 iron_nugget
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);

        // Its only output is iron_ingot, at position 0.
        let from_nuggets = recipe(&graph, "iron_ingot_from_nuggets");
        edges.set(from_nuggets, 0, EdgeState::Inert, Cause::Default);

        let live = transitively_required(&graph, &attrs, &edges);

        assert!(
            live.is_transitively_required(item(&graph, "iron_pickaxe")),
            "iron_pickaxe: required"
        );
        assert!(
            !live.is_transitively_required(item(&graph, "iron_nugget")),
            "iron_nugget: only reachable through iron_ingot_from_nuggets' inert edge"
        );
        assert!(
            live.is_transitively_required(item(&graph, "iron_ingot")),
            "iron_ingot: used by iron_pickaxe"
        );
        assert!(
            live.is_transitively_required(item(&graph, "stick")),
            "stick: used by iron_pickaxe"
        );

        assert!(
            live.is_recipe_transitively_required(recipe(&graph, "iron_pickaxe")),
            "iron_pickaxe propagates"
        );
        assert!(
            !live.is_recipe_transitively_required(from_nuggets),
            "iron_ingot_from_nuggets' only edge is inert"
        );
        assert!(
            !live.is_recipe_transitively_required(recipe(&graph, "iron_nugget")),
            "iron_nugget's output never becomes transitively required"
        );
    }

    #[test]
    fn dropped_edge_also_blocks_propagation() {
        let (graph, required_items) = parse(
            "
            required iron_pickaxe
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);
        let crafting = recipe(&graph, "iron_pickaxe");
        edges.set(crafting, 0, EdgeState::Dropped, Cause::Default);

        let live = transitively_required(&graph, &attrs, &edges);

        assert!(
            live.is_transitively_required(item(&graph, "iron_pickaxe")),
            "iron_pickaxe: required"
        );
        assert!(
            !live.is_transitively_required(item(&graph, "iron_ingot")),
            "the crafting recipe's edge is dropped"
        );
        assert!(
            !live.is_transitively_required(item(&graph, "stick")),
            "the crafting recipe's edge is dropped"
        );
        assert!(!live.is_recipe_transitively_required(crafting));
    }

    #[test]
    fn propagates_through_multiple_recipe_hops() {
        let (graph, required_items) = parse(
            "
            required iron_pickaxe
            iron_pickaxe:                      3 iron_ingot + 2 stick -> iron_pickaxe
            iron_ingot_from_smelting_raw_iron: raw_iron -> iron_ingot
            raw_iron:                          raw_iron_block -> 9 raw_iron
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        let iron_pickaxe = item(&graph, "iron_pickaxe");
        let iron_ingot = item(&graph, "iron_ingot");
        let raw_iron = item(&graph, "raw_iron");
        let raw_iron_block = item(&graph, "raw_iron_block");

        let crafting = recipe(&graph, "iron_pickaxe");
        let smelting = recipe(&graph, "iron_ingot_from_smelting_raw_iron");
        let unpacking = recipe(&graph, "raw_iron");

        assert!(
            live.is_transitively_required(raw_iron_block),
            "three hops from the required item"
        );
        assert!(live.is_transitively_required(raw_iron));
        assert!(live.is_transitively_required(iron_ingot));
        assert!(live.is_transitively_required(iron_pickaxe));
        assert!(live.is_recipe_transitively_required(crafting));
        assert!(live.is_recipe_transitively_required(smelting));
        assert!(live.is_recipe_transitively_required(unpacking));

        assert!(matches!(
            live.why(iron_pickaxe),
            Some(Why::Required(Cause::Pseudo))
        ));
        assert!(matches!(
            live.why(iron_ingot),
            Some(Why::Via { recipe, output }) if *recipe == crafting && *output == iron_pickaxe
        ));
        assert!(matches!(
            live.why(raw_iron),
            Some(Why::Via { recipe, output }) if *recipe == smelting && *output == iron_ingot
        ));
        assert!(matches!(
            live.why(raw_iron_block),
            Some(Why::Via { recipe, output }) if *recipe == unpacking && *output == raw_iron
        ));
    }

    #[test]
    fn shortest_path_gets_attributed_by_why() {
        let (graph, required_items) = parse(
            "
            required crafting_table torch
            crafting_table: 4 oak_planks -> crafting_table
            stick:          2 oak_planks -> 4 stick
            torch:          coal + stick -> 4 torch
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        let crafting_table = item(&graph, "crafting_table");
        let table_recipe = recipe(&graph, "crafting_table");
        assert!(matches!(
            live.why(item(&graph, "oak_planks")),
            Some(Why::Via { recipe, output }) if *recipe == table_recipe && *output == crafting_table
        ));
    }

    proptest! {
        #[test]
        fn matches_the_naive_fixpoint_oracle((graph, attrs, edges) in arb_graph_attrs_edges()) {
            let live = transitively_required(&graph, &attrs, &edges);
            let (oracle_item, oracle_recipe) = naive_transitively_required(&graph, &attrs, &edges);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                prop_assert_eq!(live.is_transitively_required(item), oracle_item.contains(position));
            }
            for position in 0..graph.recipes().len() {
                let recipe = RecipeIndex::new(u32::try_from(position).expect("recipe count fits u32"));
                prop_assert_eq!(live.is_recipe_transitively_required(recipe), oracle_recipe.contains(position));
            }
        }

        #[test]
        fn adding_a_required_item_never_shrinks_the_transitively_required_set(
            (graph, attrs, edges) in arb_graph_attrs_edges(),
            extra_required in 0_usize..9,
        ) {
            let extra_required_item = ItemIndex::new(
                u32::try_from(extra_required % graph.items().len()).expect("item count fits u32"),
            );
            let before = transitively_required(&graph, &attrs, &edges);

            let mut attrs_with_extra_required = attrs;
            attrs_with_extra_required.set_required(extra_required_item, Cause::Pseudo);
            let after = transitively_required(&graph, &attrs_with_extra_required, &edges);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                if before.is_transitively_required(item) {
                    prop_assert!(after.is_transitively_required(item));
                }
            }
        }

        #[test]
        fn dropping_an_edge_never_grows_the_transitively_required_set(
            (graph, attrs, edges) in arb_graph_attrs_edges(),
            edge_pick in 0_usize..27,
        ) {
            let total_edges: usize = graph.recipes().iter().map(|recipe| recipe.outputs().len()).sum();
            prop_assume!(total_edges > 0);

            let mut remaining = edge_pick % total_edges;
            let mut chosen = None;
            for (recipe_position, recipe) in graph.recipes().iter().enumerate() {
                if remaining < recipe.outputs().len() {
                    chosen = Some((recipe_position, remaining));
                    break;
                }
                remaining -= recipe.outputs().len();
            }
            let (recipe_position, output_position) =
                chosen.expect("edge_pick reduced mod total_edges always lands on a real edge");
            let recipe = RecipeIndex::new(u32::try_from(recipe_position).expect("recipe count fits u32"));
            let output = u8::try_from(output_position).expect("outputs per recipe fit u8");

            let before = transitively_required(&graph, &attrs, &edges);

            let mut edges_with_drop = edges;
            let cause = edges_with_drop.cause(recipe, output).clone();
            edges_with_drop.set(recipe, output, EdgeState::Dropped, cause);
            let after = transitively_required(&graph, &attrs, &edges_with_drop);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                if after.is_transitively_required(item) {
                    prop_assert!(before.is_transitively_required(item));
                }
            }
        }
    }
}
