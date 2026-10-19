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
                let output_index =
                    u8::try_from(output_position).expect("fewer than u8::MAX outputs per recipe");
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

#[cfg(test)]
mod tests {
    use super::Producers;
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn finds_the_recipe_and_output_position_that_produce_an_item() {
        let (graph, _roots) = parse(
            "
            root concrete
            mix: stone_dust + water -> concrete
            ",
        );
        let producers = Producers::build(&graph);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        assert_eq!(producers.of(ItemIndex::new(0)), &[(RecipeIndex::new(0), 0)]);
        assert_eq!(producers.of(ItemIndex::new(1)), &[]);
        assert_eq!(producers.of(ItemIndex::new(2)), &[]);
    }
}
