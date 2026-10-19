//!
//! Item and recipe ids are meaningless placeholders (`"item000"`,
//! `"recipe000"`, zero-padded so they stay lexicographically sorted
//! regardless of count).

use std::collections::BTreeSet;

use proptest::prelude::*;

use super::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
use crate::model::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, RecipeIndex, Slot};

/// One recipe before item positions are turned into real indices.
#[derive(Clone, Debug)]
struct RawRecipe {
    inputs: Vec<usize>,
    outputs: Vec<usize>,
}

fn arb_edge_state() -> impl Strategy<Value = EdgeState> {
    prop_oneof![
        6 => Just(EdgeState::Propagate),
        2 => Just(EdgeState::Inert),
        2 => Just(EdgeState::Dropped),
    ]
}

fn arb_raw_recipe(item_count: usize) -> impl Strategy<Value = RawRecipe> {
    let item_position = 0..item_count;
    (
        prop::collection::vec(item_position.clone(), 0..=3),
        prop::collection::vec(item_position, 0..=3),
    )
        .prop_map(|(inputs, outputs)| RawRecipe { inputs, outputs })
}

/// A random small graph, its liveness roots, and random edge states.
///
/// Up to 9 items and 9 recipes, each with up to 3 inputs/outputs.
pub(super) fn arb_graph_attrs_edges() -> impl Strategy<Value = (Graph, NodeAttrs, EdgeStates)> {
    (1_usize..=9, 0_usize..=9)
        .prop_flat_map(|(item_count, recipe_count)| {
            (
                Just(item_count),
                prop::collection::vec(arb_raw_recipe(item_count), recipe_count),
                prop::collection::vec(any::<bool>(), item_count),
            )
        })
        .prop_flat_map(|(item_count, raw_recipes, root_flags)| {
            let total_outputs: usize = raw_recipes.iter().map(|recipe| recipe.outputs.len()).sum();
            (
                Just(item_count),
                Just(raw_recipes),
                Just(root_flags),
                prop::collection::vec(arb_edge_state(), total_outputs),
            )
        })
        .prop_map(
            |(item_count, raw_recipes, root_flags, edge_state_choices)| {
                build(item_count, &raw_recipes, &root_flags, edge_state_choices)
            },
        )
}

fn build(
    item_count: usize,
    raw_recipes: &[RawRecipe],
    root_flags: &[bool],
    edge_state_choices: Vec<EdgeState>,
) -> (Graph, NodeAttrs, EdgeStates) {
    let item_index = |position: usize| {
        ItemIndex::new(u32::try_from(position).expect("item_count stays far below u32::MAX"))
    };

    let items = (0..item_count)
        .map(|position| {
            Item::new(
                format!("item{position:03}"),
                ItemKind::Item,
                BTreeSet::new(),
            )
        })
        .collect();

    let recipes = raw_recipes
        .iter()
        .enumerate()
        .map(|(recipe_position, raw)| {
            let inputs = raw
                .inputs
                .iter()
                .map(|&position| Slot::new(BTreeSet::from([item_index(position)]), None, 1, 1.0))
                .collect();
            let outputs = raw
                .outputs
                .iter()
                .map(|&position| Output::new(item_index(position), 1, 1.0))
                .collect();
            Recipe::new(
                format!("recipe{recipe_position:03}"),
                0,
                inputs,
                outputs,
                None,
                None,
            )
        })
        .collect();

    let graph = Graph::new(1, items, recipes, Vec::new(), vec!["test".to_owned()]);

    let mut edges = EdgeStates::defaults(&graph);
    let mut choices = edge_state_choices.into_iter();
    for (recipe_position, raw) in raw_recipes.iter().enumerate() {
        let recipe = RecipeIndex::new(
            u32::try_from(recipe_position).expect("recipe_count stays far below u32::MAX"),
        );
        for output_position in 0..raw.outputs.len() {
            let output =
                u8::try_from(output_position).expect("outputs per recipe stay far below u8::MAX");
            let state = choices
                .next()
                .expect("one edge state generated per output, consumed in the same order");
            edges.set(recipe, output, state, Cause::Default);
        }
    }

    let mut attrs = NodeAttrs::defaults(&graph);
    for (position, &is_root) in root_flags.iter().enumerate() {
        if is_root {
            attrs.set_root(item_index(position), Cause::Pseudo);
        }
    }

    (graph, attrs, edges)
}
