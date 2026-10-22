//! The liveness computation: which items and recipes are needed.
//!
//! ```txt
//! needed(i) ⟺
//!   i ∈ wanted OR
//!   ∃ recipe r, output o:
//!     i is an input of r AND
//!     o is an output of r AND
//!     needed(o) == true AND
//!     state(r, o) = propagate
//! ```
//! Starting from the wanted items and walking backwards through producers.
//!
//! The other alternatives, i.e. starting from non-wanted recipes with no needed
//! successors, marking them as dead, and iterating, maintains the self-sustaining
//! junk-cycles like (stone dust <-> small stone dust). Propagating neededness prevents
//! this.

use core::iter::repeat_with;
use std::collections::VecDeque;

use fixedbitset::FixedBitSet;

use super::adjacency::Producers;
use super::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
use crate::model::{Graph, ItemIndex, RecipeIndex};

/// Why an item ended up needed.
#[derive(Debug)]
pub enum Why {
    /// Needed because it's wanted directly. Also explains why it was wanted.
    Wanted(Cause),
    /// Needed because `recipe`'s output `output` is needed, with a propagating edge.
    Via {
        /// The recipe that has this item as an input.
        recipe: RecipeIndex,
        /// The needed output that made this item needed.
        output: ItemIndex,
    },
}

/// The result of [`needed`]: which items and recipes are needed, and why.
#[derive(Debug)]
pub struct Needed {
    /// `item_bits.contains(item.index())`: whether `item` is needed.
    item_bits: FixedBitSet,
    /// `recipe_bits.contains(recipe.index())`: whether `recipe` has at
    /// least one needed output over a propagating edge.
    recipe_bits: FixedBitSet,
    /// `why[item.index()]` is why that item is needed, or `None` if it's dead.
    why: Vec<Option<Why>>,
}

impl Needed {
    /// Whether `item` is needed.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was computed from.
    #[inline]
    #[must_use]
    pub fn is_needed(&self, item: ItemIndex) -> bool {
        self.item_bits.contains(item.index())
    }

    /// Whether `recipe` propagates, i.e. whether at least one of its
    /// outputs is needed over a propagating edge.
    ///
    /// # Panics
    ///
    /// If `recipe` is out of bounds for the graph this was computed from.
    #[inline]
    #[must_use]
    pub fn is_recipe_needed(&self, recipe: RecipeIndex) -> bool {
        self.recipe_bits.contains(recipe.index())
    }

    /// Why `item` is needed, or `None` if it's dead.
    #[inline]
    #[must_use]
    pub fn why(&self, item: ItemIndex) -> Option<&Why> {
        self.why.get(item.index()).and_then(Option::as_ref)
    }
}

/// Computes what's needed.
///
/// Reverse BFS from `attrs`'s wanted items, over `graph`'s recipes, and only
/// following edges `edges` marked [`EdgeState::Propagate`].
///
/// # Panics
///
/// If `attrs`, `edges` or their indices weren't built from `graph`.
#[inline]
#[must_use]
pub fn needed(graph: &Graph, attrs: &NodeAttrs, edges: &EdgeStates) -> Needed {
    let producers = Producers::build(graph);

    let mut item_bits = FixedBitSet::with_capacity(graph.items().len());
    let mut recipe_bits = FixedBitSet::with_capacity(graph.recipes().len());
    let mut why: Vec<Option<Why>> = repeat_with(|| None).take(graph.items().len()).collect();
    // TODO: Once we have multiple sets to BFS over, we probably should still
    // only have one queue (marked as "is this relevant to *anything*"),
    // coupled with a bitvec relevant[i] "is this item relevant to ruleset i?".
    // Does that make sense?
    let mut queue: VecDeque<ItemIndex> = VecDeque::new();

    for wanted in attrs.wanted_items() {
        item_bits.insert(wanted.index());
        let cause = attrs
            .wanted_cause(wanted)
            .expect("a wanted item always has a cause, set alongside it by NodeAttrs::set_wanted")
            .clone();
        *why.get_mut(wanted.index())
            .expect("wanted index in bounds for this graph") = Some(Why::Wanted(cause));
        queue.push_back(wanted);
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

    Needed {
        item_bits,
        recipe_bits,
        why,
    }
}

/// Naive reference version of [`needed`]. Iterates the fixpoint equation to a fixed
/// point by rescanning every recipe, instead of BFS.
///
/// This is only used to test [`needed`] against a structurally different implementation
/// of the equation.
#[cfg(test)]
fn naive_needed(
    graph: &Graph,
    attrs: &NodeAttrs,
    edges: &EdgeStates,
) -> (FixedBitSet, FixedBitSet) {
    let mut item_bits = FixedBitSet::with_capacity(graph.items().len());
    for position in 0..graph.items().len() {
        let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
        if attrs.is_wanted(item) {
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

    use super::{naive_needed, needed, Why};
    use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
    use crate::analysis::proptest_support::arb_graph_attrs_edges;
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn marks_wanted_items_and_their_producer_chain_needed() {
        let (graph, roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for wanted in roots {
            attrs.set_wanted(wanted, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = needed(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        // Sorted recipe order: mix=0.
        let concrete = ItemIndex::new(0);
        let stone_dust = ItemIndex::new(1);
        let water = ItemIndex::new(2);
        let mix = RecipeIndex::new(0);

        assert!(live.is_needed(concrete));
        assert!(live.is_needed(stone_dust));
        assert!(live.is_needed(water));
        assert!(live.is_recipe_needed(mix));

        assert!(matches!(
            live.why(concrete),
            Some(Why::Wanted(Cause::Pseudo))
        ));
        assert!(matches!(
            live.why(stone_dust),
            Some(Why::Via { recipe, output }) if *recipe == mix && *output == concrete
        ));
    }

    #[test]
    fn stone_dust_cycle_dies_entirely_when_nothing_wants_it() {
        let (graph, roots) = parse(
            "
            pack:   4 small_stone_dust -> stone_dust
            unpack: stone_dust -> 4 small_stone_dust
            ",
        );
        assert!(roots.is_empty(), "no root line in this fixture");
        let attrs = NodeAttrs::defaults(&graph);
        let edges = EdgeStates::defaults(&graph);

        let live = needed(&graph, &attrs, &edges);

        // Sorted item order: small_stone_dust=0, stone_dust=1.
        // Sorted recipe order: pack=0, unpack=1.
        assert!(!live.is_needed(ItemIndex::new(0)));
        assert!(!live.is_needed(ItemIndex::new(1)));
        assert!(!live.is_recipe_needed(RecipeIndex::new(0)));
        assert!(!live.is_recipe_needed(RecipeIndex::new(1)));
    }

    #[test]
    fn stone_dust_used_keeps_small_dust_dead_if_packing_is_marked_inert() {
        let (graph, roots) = parse(
            "
            root concrete
            mix:    stone_dust + water -> concrete
            pack:   4 small_stone_dust -> stone_dust
            unpack: stone_dust -> 4 small_stone_dust
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for wanted in roots {
            attrs.set_wanted(wanted, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);

        // Sorted recipe order: mix=0, pack=1, unpack=2.
        // pack's only output is stone_dust, at position 0.
        let pack = RecipeIndex::new(1);
        edges.set(pack, 0, EdgeState::Inert, Cause::Default);

        let live = needed(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, small_stone_dust=1, stone_dust=2, water=3.
        assert!(live.is_needed(ItemIndex::new(0)), "concrete: wanted");
        assert!(
            !live.is_needed(ItemIndex::new(1)),
            "small_stone_dust: only reachable through pack's inert edge"
        );
        assert!(live.is_needed(ItemIndex::new(2)), "stone_dust: used by mix");
        assert!(live.is_needed(ItemIndex::new(3)), "water: used by mix");

        assert!(live.is_recipe_needed(RecipeIndex::new(0)), "mix propagates");
        assert!(!live.is_recipe_needed(pack), "pack's only edge is inert");
        assert!(
            !live.is_recipe_needed(RecipeIndex::new(2)),
            "unpack's output never becomes needed"
        );
    }

    #[test]
    fn dropped_edge_also_blocks_propagation() {
        let (graph, roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for wanted in roots {
            attrs.set_wanted(wanted, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);
        edges.set(RecipeIndex::new(0), 0, EdgeState::Dropped, Cause::Default);

        let live = needed(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        assert!(live.is_needed(ItemIndex::new(0)), "concrete: wanted");
        assert!(!live.is_needed(ItemIndex::new(1)), "mix's edge is dropped");
        assert!(!live.is_needed(ItemIndex::new(2)), "mix's edge is dropped");
        assert!(!live.is_recipe_needed(RecipeIndex::new(0)));
    }

    #[test]
    fn propagates_through_multiple_recipe_hops() {
        let (graph, roots) = parse(
            "
            root plate
            press: ingot -> plate
            smelt: ore -> ingot
            mine:  dirt -> ore
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for wanted in roots {
            attrs.set_wanted(wanted, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = needed(&graph, &attrs, &edges);

        // Sorted item order: dirt=0, ingot=1, ore=2, plate=3.
        let dirt = ItemIndex::new(0);
        let ingot = ItemIndex::new(1);
        let ore = ItemIndex::new(2);
        let plate = ItemIndex::new(3);

        // Sorted recipe order: mine=0, press=1, smelt=2.
        let mine = RecipeIndex::new(0);
        let press = RecipeIndex::new(1);
        let smelt = RecipeIndex::new(2);

        assert!(live.is_needed(dirt), "three hops from the wanted item");
        assert!(live.is_needed(ingot));
        assert!(live.is_needed(ore));
        assert!(live.is_needed(plate));
        assert!(live.is_recipe_needed(mine));
        assert!(live.is_recipe_needed(press));
        assert!(live.is_recipe_needed(smelt));

        assert!(matches!(live.why(plate), Some(Why::Wanted(Cause::Pseudo))));
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
        let (graph, roots) = parse(
            "
            root a
            root b
            short: shared -> a
            long1: mid -> b
            long2: shared -> mid
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for wanted in roots {
            attrs.set_wanted(wanted, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = needed(&graph, &attrs, &edges);

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
            let live = needed(&graph, &attrs, &edges);
            let (oracle_needed_item, oracle_needed_recipe) = naive_needed(&graph, &attrs, &edges);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                prop_assert_eq!(live.is_needed(item), oracle_needed_item.contains(position));
            }
            for position in 0..graph.recipes().len() {
                let recipe = RecipeIndex::new(u32::try_from(position).expect("recipe count fits u32"));
                prop_assert_eq!(live.is_recipe_needed(recipe), oracle_needed_recipe.contains(position));
            }
        }

        #[test]
        fn adding_a_wanted_item_never_shrinks_the_needed_set(
            (graph, attrs, edges) in arb_graph_attrs_edges(),
            extra_wanted in 0_usize..9,
        ) {
            let extra_wanted_item = ItemIndex::new(
                u32::try_from(extra_wanted % graph.items().len()).expect("item count fits u32"),
            );
            let before = needed(&graph, &attrs, &edges);

            let mut attrs_with_extra_wanted = attrs;
            attrs_with_extra_wanted.set_wanted(extra_wanted_item, Cause::Pseudo);
            let after = needed(&graph, &attrs_with_extra_wanted, &edges);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                if before.is_needed(item) {
                    prop_assert!(after.is_needed(item));
                }
            }
        }

        #[test]
        fn dropping_an_edge_never_grows_the_needed_set(
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

            let before = needed(&graph, &attrs, &edges);

            let mut edges_with_drop = edges;
            let cause = edges_with_drop.cause(recipe, output).clone();
            edges_with_drop.set(recipe, output, EdgeState::Dropped, cause);
            let after = needed(&graph, &attrs, &edges_with_drop);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                if after.is_needed(item) {
                    prop_assert!(before.is_needed(item));
                }
            }
        }
    }
}
