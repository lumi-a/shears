//! Propagation lives *only* on `recipe -> output` edges, not on inputs.

use fixedbitset::FixedBitSet;
use serde::{Deserialize, Serialize};

use crate::model::{Graph, ItemIndex, RecipeIndex};

/// What a single `(recipe, output)` edge does for liveness propagation.
///
/// TODO: This probably won't be sufficient later on. Later on, we'll probably want
/// to have things like "This edge is only useful in LV, but not in MV+", or
/// "This edge is only useful for builders, but not for technical aspects", and
/// these states should be specified by the user, i.e. not known at compile-time.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EdgeState {
    /// Normal: an alive output on this edge makes the recipe's inputs alive too.
    Propagate,
    /// Output exists, but doesn't makes the recipe's inputs alive on its own.
    Inert,
    /// Treated as if the recipe couldn't produce this output at all.
    /// Usually, this state is assigned for `(recipe, *)`, to drop the recipe entirely.
    Dropped,
}

/// Whether a recipe should be considered for visibility regardless of liveness.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Show {
    /// Shown if it has a visible output on a non-dropped edge.
    Auto,
    /// Always a visibility candidate, regardless of its outputs' states.
    Always,
    /// Never a visibility candidate, regardless of its outputs' states.
    Never,
}

/// Why a state was assigned.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Cause {
    /// Nothing has touched this edge/item.
    Default,
    /// A pseudo-item root.
    ///
    /// TODO: Do we ever actually need this?
    Pseudo,
    /// A rule from `rules.toml`.
    ///
    /// `file` and `index` locate which rule, for `explain::why`.
    Rule {
        /// Which rules file, in load order (presets first, then local).
        file: u16,
        /// The rule's position within that file.
        index: u32,
    },
    /// A decision from `decisions.toml`, by its position in the file.
    Decision {
        /// The decision's position in `decisions.toml`.
        index: u32,
    },
}

/// For each `(recipe, output)` edge, a state and corresponding cause.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeStates {
    /// `state[recipe.index()][output]` is that edge's current [`EdgeState`].
    state: Vec<Vec<EdgeState>>,
    /// `cause[recipe.index()][output]` is why `state[recipe.index()][output]` is what it
    /// is.
    cause: Vec<Vec<Cause>>,
}

impl EdgeStates {
    /// Every edge starts at [`EdgeState::Propagate`]/[`Cause::Default`], shaped like
    /// `graph`.
    #[inline]
    #[must_use]
    pub fn defaults(graph: &Graph) -> Self {
        let state = graph
            .recipes()
            .iter()
            .map(|recipe| vec![EdgeState::Propagate; recipe.outputs().len()])
            .collect();
        let cause = graph
            .recipes()
            .iter()
            .map(|recipe| recipe.outputs().iter().map(|_| Cause::Default).collect())
            .collect();
        Self { state, cause }
    }

    /// The state of `recipe`'s `output`-th output edge.
    ///
    /// # Panics
    ///
    /// If `output` is out of bounds for `recipe`'s outputs.
    #[inline]
    #[must_use]
    pub fn state(&self, recipe: RecipeIndex, output: u8) -> EdgeState {
        *self
            .state
            .get(recipe.index())
            .and_then(|outputs| outputs.get(usize::from(output)))
            .expect("output index in bounds for this recipe")
    }

    /// All of `recipe`'s output edge states.
    ///
    /// # Panics
    ///
    /// If `recipe` is out of bounds for this graph.
    #[inline]
    #[must_use]
    pub fn states(&self, recipe: RecipeIndex) -> &[EdgeState] {
        self.state
            .get(recipe.index())
            .expect("recipe index in bounds for this graph")
    }

    /// The cause of `recipe`'s `output`-th output edge.
    ///
    /// # Panics
    ///
    /// If `output` is out of bounds for `recipe`'s outputs.
    #[inline]
    #[must_use]
    pub fn cause(&self, recipe: RecipeIndex, output: u8) -> &Cause {
        self.cause
            .get(recipe.index())
            .and_then(|outputs| outputs.get(usize::from(output)))
            .expect("output index in bounds for this recipe")
    }

    /// Sets both the state and its cause for one edge.
    ///
    /// # Panics
    ///
    /// If `output` is out of bounds for `recipe`'s outputs.
    #[inline]
    pub fn set(&mut self, recipe: RecipeIndex, output: u8, state: EdgeState, cause: Cause) {
        *self
            .state
            .get_mut(recipe.index())
            .and_then(|outputs| outputs.get_mut(usize::from(output)))
            .expect("output index in bounds for this recipe") = state;
        *self
            .cause
            .get_mut(recipe.index())
            .and_then(|outputs| outputs.get_mut(usize::from(output)))
            .expect("output index in bounds for this recipe") = cause;
    }
}

/// Per-item annotations computed before edge states.
///
/// Material and form per item, and per-recipe attributes like [`Show`].
#[derive(Clone, Debug)]
pub struct NodeAttrs {
    /// `root.contains(item.index())`: whether `item` is a liveness root.
    root: FixedBitSet,
    /// `root_cause[item.index()]` is why `item` is a root, if it is one.
    root_cause: Vec<Option<Cause>>,
    /// `world_source.contains(item.index())`: whether `item` is obtainable
    /// in-world (ore, log, gravel, mob drop, ...).
    ///
    /// Not inferred from having no producing recipe, instead opt-in
    /// via [`Self::set_world_source`].
    world_source: FixedBitSet,
}

impl NodeAttrs {
    /// Nothing is a root or a world source yet.
    #[inline]
    #[must_use]
    pub fn defaults(graph: &Graph) -> Self {
        let item_count = graph.items().len();
        Self {
            root: FixedBitSet::with_capacity(item_count),
            root_cause: vec![None; item_count],
            world_source: FixedBitSet::with_capacity(item_count),
        }
    }

    /// Whether `item` is currently a liveness root.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was built from.
    #[inline]
    #[must_use]
    pub fn is_root(&self, item: ItemIndex) -> bool {
        self.root.contains(item.index())
    }

    /// Why `item` is a root, if it is one.
    #[inline]
    #[must_use]
    pub fn root_cause(&self, item: ItemIndex) -> Option<&Cause> {
        self.root_cause.get(item.index()).and_then(Option::as_ref)
    }

    /// Every item currently marked as a liveness root.
    ///
    /// # Panics
    ///
    /// If this graph has more than `u32::MAX` items.
    #[inline]
    pub fn roots(&self) -> impl Iterator<Item = ItemIndex> + '_ {
        self.root.ones().map(|position| {
            ItemIndex::new(u32::try_from(position).expect("fewer than u32::MAX items"))
        })
    }

    /// Marks `item` as a liveness root, and why.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was built from.
    #[inline]
    pub fn set_root(&mut self, item: ItemIndex, cause: Cause) {
        self.root.insert(item.index());
        *self
            .root_cause
            .get_mut(item.index())
            .expect("item index in bounds for this graph") = Some(cause);
    }

    /// Whether `item` is a world source.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was built from.
    #[inline]
    #[must_use]
    pub fn is_world_source(&self, item: ItemIndex) -> bool {
        self.world_source.contains(item.index())
    }

    /// Marks `item` as a world source.
    ///
    /// # Panics
    ///
    /// If `item` is out of bounds for the graph this was built from.
    #[inline]
    pub fn set_world_source(&mut self, item: ItemIndex) {
        self.world_source.insert(item.index());
    }
}

#[cfg(test)]
mod tests {
    use super::{Cause, EdgeState, EdgeStates, NodeAttrs};
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn defaults_are_propagate_with_no_cause_and_shaped_like_the_graph() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let edges = EdgeStates::defaults(&graph);

        assert_eq!(edges.states(RecipeIndex::new(0)), &[EdgeState::Propagate]);
        assert_eq!(edges.state(RecipeIndex::new(0), 0), EdgeState::Propagate);
        assert!(matches!(
            edges.cause(RecipeIndex::new(0), 0),
            Cause::Default
        ));
    }

    #[test]
    fn set_updates_state_and_cause_together() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut edges = EdgeStates::defaults(&graph);

        edges.set(
            RecipeIndex::new(0),
            0,
            EdgeState::Dropped,
            Cause::Decision { index: 3 },
        );

        assert_eq!(edges.state(RecipeIndex::new(0), 0), EdgeState::Dropped);
        assert!(matches!(
            edges.cause(RecipeIndex::new(0), 0),
            Cause::Decision { index: 3 }
        ));
    }

    #[test]
    #[should_panic(expected = "output index in bounds")]
    fn state_panics_on_an_out_of_bounds_output() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let edges = EdgeStates::defaults(&graph);

        let _state = edges.state(RecipeIndex::new(0), 1);
    }

    #[test]
    fn defaults_root_and_source_nothing_even_producerless_items() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let attrs = NodeAttrs::defaults(&graph);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        assert!(!attrs.is_root(ItemIndex::new(0)));
        assert!(attrs.root_cause(ItemIndex::new(0)).is_none());

        // stone_dust and water have no producer in this fixture.
        assert!(!attrs.is_world_source(ItemIndex::new(0)));
        assert!(!attrs.is_world_source(ItemIndex::new(1)));
        assert!(!attrs.is_world_source(ItemIndex::new(2)));
    }

    #[test]
    fn set_world_source_marks_only_that_item() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);

        attrs.set_world_source(ItemIndex::new(1));

        assert!(attrs.is_world_source(ItemIndex::new(1)));
        assert!(!attrs.is_world_source(ItemIndex::new(0)));
        assert!(!attrs.is_world_source(ItemIndex::new(2)));
    }

    #[test]
    fn set_root_marks_the_item_and_records_the_cause() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);

        attrs.set_root(ItemIndex::new(1), Cause::Pseudo);

        assert!(attrs.is_root(ItemIndex::new(1)));
        assert!(matches!(
            attrs.root_cause(ItemIndex::new(1)),
            Some(Cause::Pseudo)
        ));
        assert!(
            !attrs.is_root(ItemIndex::new(0)),
            "unrelated items stay unrooted"
        );
    }
}
