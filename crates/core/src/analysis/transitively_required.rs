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
//! required successors, marking them as dead, and iterating, maintains the
//! self-sustaining junk-cycles like (stone dust <-> small stone dust). Propagating
//! transitive requirement prevents this.

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

/// The result of [`transitively_required`]: which items and recipes are transitively
/// required, and why.
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
}

/// Computes what's transitively required.
///
/// Reverse BFS from `attrs`'s required items, over `graph`'s recipes, and only
/// following edges `edges` marked [`EdgeState::Propagate`].
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

/// Naive reference version of [`transitively_required`]. Iterates the fixpoint equation
/// to a fixed point by rescanning every recipe, instead of BFS.
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
            let recipe_index =
                RecipeIndex::new(u32::try_from(recipe_position).expect("recipe count fits u32"));
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

    use super::{naive_transitively_required, transitively_required, Why};
    use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
    use crate::analysis::proptest_support::arb_graph_attrs_edges;
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn marks_required_items_and_their_producer_chain_transitively_required() {
        let (graph, required_items) = parse(
            "
            required concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        // Sorted recipe order: mix=0.
        let concrete = ItemIndex::new(0);
        let stone_dust = ItemIndex::new(1);
        let water = ItemIndex::new(2);
        let mix = RecipeIndex::new(0);

        assert!(live.is_transitively_required(concrete));
        assert!(live.is_transitively_required(stone_dust));
        assert!(live.is_transitively_required(water));
        assert!(live.is_recipe_transitively_required(mix));

        assert!(matches!(
            live.why(concrete),
            Some(Why::Required(Cause::Pseudo))
        ));
        assert!(matches!(
            live.why(stone_dust),
            Some(Why::Via { recipe, output }) if *recipe == mix && *output == concrete
        ));
    }

    #[test]
    fn stone_dust_cycle_dies_entirely_when_nothing_is_required() {
        let (graph, required_items) = parse(
            "
            pack:   4 small_stone_dust -> stone_dust
            unpack: stone_dust -> 4 small_stone_dust
            ",
        );
        assert!(
            required_items.is_empty(),
            "no required line in this fixture"
        );
        let attrs = NodeAttrs::defaults(&graph);
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: small_stone_dust=0, stone_dust=1.
        // Sorted recipe order: pack=0, unpack=1.
        assert!(!live.is_transitively_required(ItemIndex::new(0)));
        assert!(!live.is_transitively_required(ItemIndex::new(1)));
        assert!(!live.is_recipe_transitively_required(RecipeIndex::new(0)));
        assert!(!live.is_recipe_transitively_required(RecipeIndex::new(1)));
    }

    #[test]
    fn stone_dust_used_keeps_small_dust_dead_if_packing_is_marked_inert() {
        let (graph, required_items) = parse(
            "
            required concrete
            mix:    stone_dust + water -> concrete
            pack:   4 small_stone_dust -> stone_dust
            unpack: stone_dust -> 4 small_stone_dust
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);

        // Sorted recipe order: mix=0, pack=1, unpack=2.
        // pack's only output is stone_dust, at position 0.
        let pack = RecipeIndex::new(1);
        edges.set(pack, 0, EdgeState::Inert, Cause::Default);

        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, small_stone_dust=1, stone_dust=2, water=3.
        assert!(
            live.is_transitively_required(ItemIndex::new(0)),
            "concrete: required"
        );
        assert!(
            !live.is_transitively_required(ItemIndex::new(1)),
            "small_stone_dust: only reachable through pack's inert edge"
        );
        assert!(
            live.is_transitively_required(ItemIndex::new(2)),
            "stone_dust: used by mix"
        );
        assert!(
            live.is_transitively_required(ItemIndex::new(3)),
            "water: used by mix"
        );

        assert!(
            live.is_recipe_transitively_required(RecipeIndex::new(0)),
            "mix propagates"
        );
        assert!(
            !live.is_recipe_transitively_required(pack),
            "pack's only edge is inert"
        );
        assert!(
            !live.is_recipe_transitively_required(RecipeIndex::new(2)),
            "unpack's output never becomes transitively required"
        );
    }

    #[test]
    fn dropped_edge_also_blocks_propagation() {
        let (graph, required_items) = parse(
            "
            required concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);
        edges.set(RecipeIndex::new(0), 0, EdgeState::Dropped, Cause::Default);

        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        assert!(
            live.is_transitively_required(ItemIndex::new(0)),
            "concrete: required"
        );
        assert!(
            !live.is_transitively_required(ItemIndex::new(1)),
            "mix's edge is dropped"
        );
        assert!(
            !live.is_transitively_required(ItemIndex::new(2)),
            "mix's edge is dropped"
        );
        assert!(!live.is_recipe_transitively_required(RecipeIndex::new(0)));
    }

    #[test]
    fn propagates_through_multiple_recipe_hops() {
        let (graph, required_items) = parse(
            "
            required plate
            press: ingot -> plate
            smelt: ore -> ingot
            mine:  dirt -> ore
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: dirt=0, ingot=1, ore=2, plate=3.
        let dirt = ItemIndex::new(0);
        let ingot = ItemIndex::new(1);
        let ore = ItemIndex::new(2);
        let plate = ItemIndex::new(3);

        // Sorted recipe order: mine=0, press=1, smelt=2.
        let mine = RecipeIndex::new(0);
        let press = RecipeIndex::new(1);
        let smelt = RecipeIndex::new(2);

        assert!(
            live.is_transitively_required(dirt),
            "three hops from the required item"
        );
        assert!(live.is_transitively_required(ingot));
        assert!(live.is_transitively_required(ore));
        assert!(live.is_transitively_required(plate));
        assert!(live.is_recipe_transitively_required(mine));
        assert!(live.is_recipe_transitively_required(press));
        assert!(live.is_recipe_transitively_required(smelt));

        assert!(matches!(
            live.why(plate),
            Some(Why::Required(Cause::Pseudo))
        ));
        assert!(matches!(
            live.why(ingot),
            Some(Why::Via { recipe, output }) if *recipe == press && *output == plate
        ));
        assert!(matches!(
            live.why(ore),
            Some(Why::Via { recipe, output }) if *recipe == smelt && *output == ingot
        ));
        assert!(matches!(
            live.why(dirt),
            Some(Why::Via { recipe, output }) if *recipe == mine && *output == ore
        ));
    }

    #[test]
    fn shortest_path_gets_attributed_by_why() {
        let (graph, required_items) = parse(
            "
            required a
            required b
            short: shared -> a
            long1: mid -> b
            long2: shared -> mid
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: a=0, b=1, mid=2, shared=3.
        let a = ItemIndex::new(0);
        let shared = ItemIndex::new(3);
        // Sorted recipe order: long1=0, long2=1, short=2.
        let short = RecipeIndex::new(2);

        assert!(matches!(
            live.why(shared),
            Some(Why::Via { recipe, output }) if *recipe == short && *output == a
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
