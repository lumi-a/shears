//! ```txt
//! alive(i) ⟺
//!   i ∈ roots OR
//!   ∃ recipe r, output o:
//!     i is an input of r AND
//!     o is an output of r AND
//!     alive(o) == true AND
//!     state(r, o) = propagate
//! ```
//! Starting from roots and walking backwards through producers.
//!
//! The other alternatives, i.e. starting from non-root recipes with no alive successors,
//! marking them as dead, and iterating, maintains the self-sustaining junk-cycles like
//! (stone dust <-> small stone dust). Propagating liveness prevents this.

use core::iter::repeat_with;
use std::collections::VecDeque;

use fixedbitset::FixedBitSet;

use super::adjacency::Producers;
use super::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
use crate::model::{Graph, ItemIndex, RecipeIndex};

/// Why an item ended up alive.
#[derive(Debug)]
pub enum Why {
    /// Alive because it's a root. Also explains why it was rooted.
    Root(Cause),
    /// Alive because `recipe`'s output `output` is alive, with a propagating edge.
    Via {
        /// The recipe that has this item as an input.
        recipe: RecipeIndex,
        /// The alive output that made this item alive.
        output: ItemIndex,
    },
}

/// The result of [`liveness`]: which items and recipes are alive, and why.
#[derive(Debug)]
pub struct Liveness {
    /// `alive_item.contains(item.index())`: whether `item` is alive.
    alive_item: FixedBitSet,
    /// `alive_recipe.contains(recipe.index())`: whether `recipe` has at
    /// least one alive output over a propagating edge.
    alive_recipe: FixedBitSet,
    /// `why[item.index()]` is why that item is alive, or `None` if it's dead.
    why: Vec<Option<Why>>,
}

impl Liveness {
    /// Whether `item` is alive.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was computed from.
    #[inline]
    #[must_use]
    pub fn is_alive(&self, item: ItemIndex) -> bool {
        self.alive_item.contains(item.index())
    }

    /// Whether `recipe` propagates, i.e. whether at least one of its
    /// outputs is alive over a propagating edge.
    ///
    /// # Panics
    ///
    /// If `recipe` is out of bounds for the graph this was computed from.
    #[inline]
    #[must_use]
    pub fn is_recipe_alive(&self, recipe: RecipeIndex) -> bool {
        self.alive_recipe.contains(recipe.index())
    }

    /// Why `item` is alive, or `None` if it's dead.
    #[inline]
    #[must_use]
    pub fn why(&self, item: ItemIndex) -> Option<&Why> {
        self.why.get(item.index()).and_then(Option::as_ref)
    }
}

/// Computes liveness.
///
/// Reverse BFS from `attrs`'s roots, over `graph`'s recipes, and only
/// following edges `edges` marked [`EdgeState::Propagate`].
///
/// # Panics
///
/// If `attrs`, `edges` or their indices weren't built from `graph`.
#[inline]
#[must_use]
pub fn liveness(graph: &Graph, attrs: &NodeAttrs, edges: &EdgeStates) -> Liveness {
    let producers = Producers::build(graph);

    let mut alive_item = FixedBitSet::with_capacity(graph.items().len());
    let mut alive_recipe = FixedBitSet::with_capacity(graph.recipes().len());
    let mut why: Vec<Option<Why>> = repeat_with(|| None).take(graph.items().len()).collect();
    // TODO: Once we have multiple sets to BFS over, we probably should still
    // only have one queue (marked as "is this relevant to *anything*"),
    // coupled with a bitvec relevant[i] "is this item relevant to ruleset i?".
    // Does that make sense?
    let mut queue: VecDeque<ItemIndex> = VecDeque::new();

    for root in attrs.roots() {
        alive_item.insert(root.index());
        let cause = attrs
            .root_cause(root)
            .expect("a root always has a cause, set alongside it by NodeAttrs::set_root")
            .clone();
        *why.get_mut(root.index())
            .expect("root index in bounds for this graph") = Some(Why::Root(cause));
        queue.push_back(root);
    }

    while let Some(output_item) = queue.pop_front() {
        for &(recipe, output) in producers.of(output_item) {
            if edges.state(recipe, output) != EdgeState::Propagate {
                continue;
            }
            if alive_recipe.contains(recipe.index()) {
                continue;
            }
            alive_recipe.insert(recipe.index());

            let recipe_data = graph
                .recipes()
                .get(recipe.index())
                .expect("recipe index in bounds for this graph");
            for slot in recipe_data.inputs() {
                for &member in slot.members() {
                    if alive_item.contains(member.index()) {
                        continue;
                    }
                    alive_item.insert(member.index());
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

    Liveness {
        alive_item,
        alive_recipe,
        why,
    }
}

/// Naive reference liveness. Iterates the fixpoint equation to a fixed point by
/// rescanning every recipe, instead of BFS.
///
/// This is only used to test [`liveness`] against a structurally different implementation
/// of the equation.
#[cfg(test)]
fn naive_liveness(
    graph: &Graph,
    attrs: &NodeAttrs,
    edges: &EdgeStates,
) -> (FixedBitSet, FixedBitSet) {
    let mut alive_item = FixedBitSet::with_capacity(graph.items().len());
    for position in 0..graph.items().len() {
        let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
        if attrs.is_root(item) {
            alive_item.insert(position);
        }
    }
    let mut alive_recipe = FixedBitSet::with_capacity(graph.recipes().len());

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
                if !alive_item.contains(output.item().index()) {
                    continue;
                }
                if !alive_recipe.contains(recipe_position) {
                    alive_recipe.insert(recipe_position);
                    changed = true;
                }
                for slot in recipe.inputs() {
                    for &member in slot.members() {
                        if !alive_item.contains(member.index()) {
                            alive_item.insert(member.index());
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

    (alive_item, alive_recipe)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{liveness, naive_liveness, Why};
    use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
    use crate::analysis::proptest_support::arb_graph_attrs_edges;
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn marks_roots_and_their_producer_chain_alive() {
        let (graph, roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for root in roots {
            attrs.set_root(root, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = liveness(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        // Sorted recipe order: mix=0.
        let concrete = ItemIndex::new(0);
        let stone_dust = ItemIndex::new(1);
        let water = ItemIndex::new(2);
        let mix = RecipeIndex::new(0);

        assert!(live.is_alive(concrete));
        assert!(live.is_alive(stone_dust));
        assert!(live.is_alive(water));
        assert!(live.is_recipe_alive(mix));

        assert!(matches!(live.why(concrete), Some(Why::Root(Cause::Pseudo))));
        assert!(matches!(
            live.why(stone_dust),
            Some(Why::Via { recipe, output }) if *recipe == mix && *output == concrete
        ));
    }

    #[test]
    fn stone_dust_cycle_dies_entirely_when_nothing_roots_it() {
        let (graph, roots) = parse(
            "
            pack:   4 small_stone_dust -> stone_dust
            unpack: stone_dust -> 4 small_stone_dust
            ",
        );
        assert!(roots.is_empty(), "no root line in this fixture");
        let attrs = NodeAttrs::defaults(&graph);
        let edges = EdgeStates::defaults(&graph);

        let live = liveness(&graph, &attrs, &edges);

        // Sorted item order: small_stone_dust=0, stone_dust=1.
        // Sorted recipe order: pack=0, unpack=1.
        assert!(!live.is_alive(ItemIndex::new(0)));
        assert!(!live.is_alive(ItemIndex::new(1)));
        assert!(!live.is_recipe_alive(RecipeIndex::new(0)));
        assert!(!live.is_recipe_alive(RecipeIndex::new(1)));
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
        for root in roots {
            attrs.set_root(root, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);

        // Sorted recipe order: mix=0, pack=1, unpack=2.
        // pack's only output is stone_dust, at position 0.
        let pack = RecipeIndex::new(1);
        edges.set(pack, 0, EdgeState::Inert, Cause::Default);

        let live = liveness(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, small_stone_dust=1, stone_dust=2, water=3.
        assert!(live.is_alive(ItemIndex::new(0)), "concrete: root");
        assert!(
            !live.is_alive(ItemIndex::new(1)),
            "small_stone_dust: only reachable through pack's inert edge"
        );
        assert!(live.is_alive(ItemIndex::new(2)), "stone_dust: used by mix");
        assert!(live.is_alive(ItemIndex::new(3)), "water: used by mix");

        assert!(live.is_recipe_alive(RecipeIndex::new(0)), "mix propagates");
        assert!(!live.is_recipe_alive(pack), "pack's only edge is inert");
        assert!(
            !live.is_recipe_alive(RecipeIndex::new(2)),
            "unpack's output never becomes alive"
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
        for root in roots {
            attrs.set_root(root, Cause::Pseudo);
        }
        let mut edges = EdgeStates::defaults(&graph);
        edges.set(RecipeIndex::new(0), 0, EdgeState::Dropped, Cause::Default);

        let live = liveness(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        assert!(live.is_alive(ItemIndex::new(0)), "concrete: root");
        assert!(!live.is_alive(ItemIndex::new(1)), "mix's edge is dropped");
        assert!(!live.is_alive(ItemIndex::new(2)), "mix's edge is dropped");
        assert!(!live.is_recipe_alive(RecipeIndex::new(0)));
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
        for root in roots {
            attrs.set_root(root, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = liveness(&graph, &attrs, &edges);

        // Sorted item order: dirt=0, ingot=1, ore=2, plate=3.
        let dirt = ItemIndex::new(0);
        let ingot = ItemIndex::new(1);
        let ore = ItemIndex::new(2);
        let plate = ItemIndex::new(3);

        // Sorted recipe order: mine=0, press=1, smelt=2.
        let mine = RecipeIndex::new(0);
        let press = RecipeIndex::new(1);
        let smelt = RecipeIndex::new(2);

        assert!(live.is_alive(dirt), "three hops from the root");
        assert!(live.is_alive(ingot));
        assert!(live.is_alive(ore));
        assert!(live.is_alive(plate));
        assert!(live.is_recipe_alive(mine));
        assert!(live.is_recipe_alive(press));
        assert!(live.is_recipe_alive(smelt));

        assert!(matches!(live.why(plate), Some(Why::Root(Cause::Pseudo))));
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
        for root in roots {
            attrs.set_root(root, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);

        let live = liveness(&graph, &attrs, &edges);

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
            let live = liveness(&graph, &attrs, &edges);
            let (oracle_alive_item, oracle_alive_recipe) = naive_liveness(&graph, &attrs, &edges);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                prop_assert_eq!(live.is_alive(item), oracle_alive_item.contains(position));
            }
            for position in 0..graph.recipes().len() {
                let recipe = RecipeIndex::new(u32::try_from(position).expect("recipe count fits u32"));
                prop_assert_eq!(live.is_recipe_alive(recipe), oracle_alive_recipe.contains(position));
            }
        }

        #[test]
        fn adding_a_root_never_shrinks_the_alive_set(
            (graph, attrs, edges) in arb_graph_attrs_edges(),
            extra_root in 0_usize..9,
        ) {
            let extra_root_item = ItemIndex::new(
                u32::try_from(extra_root % graph.items().len()).expect("item count fits u32"),
            );
            let before = liveness(&graph, &attrs, &edges);

            let mut attrs_with_extra_root = attrs;
            attrs_with_extra_root.set_root(extra_root_item, Cause::Pseudo);
            let after = liveness(&graph, &attrs_with_extra_root, &edges);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                if before.is_alive(item) {
                    prop_assert!(after.is_alive(item));
                }
            }
        }

        #[test]
        fn dropping_an_edge_never_grows_the_alive_set(
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

            let before = liveness(&graph, &attrs, &edges);

            let mut edges_with_drop = edges;
            let cause = edges_with_drop.cause(recipe, output).clone();
            edges_with_drop.set(recipe, output, EdgeState::Dropped, cause);
            let after = liveness(&graph, &attrs, &edges_with_drop);

            for position in 0..graph.items().len() {
                let item = ItemIndex::new(u32::try_from(position).expect("item count fits u32"));
                if after.is_alive(item) {
                    prop_assert!(before.is_alive(item));
                }
            }
        }
    }
}
