//! A tiny textual DSL for hand-written test graphs. Test-only.
//!
//! ```text required concrete mix:    stone_dust + water -> concrete pack:   4
//! small_stone_dust -> stone_dust unpack: stone_dust -> 4 small_stone_dust ```
//!
//! One `required` line lists required items, space-separated. Every other non-blank line is a recipe: A name, `:`, `+`-separated inputs, `->`, then `+`-separated outputs.
//!
//! Each input or output is `[count] item`, the count defaulting to 1. Lines are trimmed before parsing.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, Slot};

/// One `[count] item` term, before its item name is resolved to an index.
struct RawTerm {
    /// The item's name.
    name: String,
    /// The count. Defaults to 1 if left out.
    count: u32,
}

/// One recipe line, before item names are resolved to indices.
struct RawRecipe {
    /// The recipe's name, taken verbatim as its id.
    name: String,
    /// Inputs, left of `->`.
    inputs: Vec<RawTerm>,
    /// Outputs, right of `->`.
    outputs: Vec<RawTerm>,
}

/// One parsed, not-yet-resolved line.
enum RawLine {
    /// A `required` line's item names.
    Required(Vec<String>),
    /// A recipe line.
    Recipe(RawRecipe),
}

/// Parses a `[count] item` term.
///
/// # Panics
///
/// If the term is empty, or has a count that doesn't parse as a `u32`.
#[expect(clippy::panic, reason = "This is only used in tests, not user-facing.")]
fn parse_term(term: &str) -> RawTerm {
    let trimmed = term.trim();
    let (count, name) = match trimmed.split_once(char::is_whitespace) {
        Some((head, rest)) => (
            head.parse()
                .unwrap_or_else(|_| panic!("expected a count before {rest:?}, found {head:?}")),
            rest.trim(),
        ),
        None => (1, trimmed),
    };
    assert!(!name.is_empty(), "empty item name in term {trimmed:?}");
    RawTerm {
        name: name.to_owned(),
        count,
    }
}

/// Parses the `+`-separated terms on one side of a recipe line.
///
/// Empty (a recipe with no inputs, or no outputs) is valid: a pure sink or a spontaneous
/// producer. `"".split('+')` would otherwise yield one empty term instead of zero.
fn parse_terms(side: &str) -> Vec<RawTerm> {
    let trimmed = side.trim();
    if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.split('+').map(parse_term).collect()
    }
}

/// Parses one non-blank, already-trimmed line.
///
/// # Panics
///
/// If a recipe line has no `:` or no `->`.
#[expect(clippy::panic, reason = "This is only used in tests, not user-facing.")]
fn parse_line(line: &str) -> RawLine {
    if let Some(rest) = line.strip_prefix("required") {
        // No names is valid too: a graph with nothing required at all, where everything
        // is dead.
        return RawLine::Required(rest.split_whitespace().map(str::to_owned).collect());
    }

    let (name, rest) = line
        .split_once(':')
        .unwrap_or_else(|| panic!("recipe line missing ':': {line:?}"));
    let (inputs, outputs) = rest
        .split_once("->")
        .unwrap_or_else(|| panic!("recipe line missing '->': {line:?}"));

    RawLine::Recipe(RawRecipe {
        name: name.trim().to_owned(),
        inputs: parse_terms(inputs),
        outputs: parse_terms(outputs),
    })
}

/// Parses the DSL into a graph and its required items.
///
/// # Panics
///
/// On any malformed line.
#[expect(clippy::panic, reason = "This is only used in tests, not user-facing.")]
#[expect(
    clippy::redundant_pub_crate,
    reason = "crate-visible on purpose: unit tests outside this module need to reach it"
)]
pub(crate) fn parse(source: &str) -> (Graph, BTreeSet<ItemIndex>) {
    let lines: Vec<RawLine> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(parse_line)
        .collect();

    let mut names: BTreeSet<String> = BTreeSet::new();
    for line in &lines {
        match line {
            RawLine::Required(required_names) => names.extend(required_names.iter().cloned()),
            RawLine::Recipe(recipe) => {
                names.extend(recipe.inputs.iter().map(|term| term.name.clone()));
                names.extend(recipe.outputs.iter().map(|term| term.name.clone()));
            }
        }
    }

    let ids: Vec<String> = names.into_iter().collect();
    let index: BTreeMap<String, ItemIndex> = ids
        .iter()
        .enumerate()
        .map(|(position, name)| {
            let item_position = u32::try_from(position).expect("fewer than u32::MAX test items");
            (name.clone(), ItemIndex::new(item_position))
        })
        .collect();
    let resolve = |name: &str| -> ItemIndex {
        *index
            .get(name)
            .unwrap_or_else(|| panic!("unknown item {name:?}"))
    };

    let mut required: BTreeSet<ItemIndex> = BTreeSet::new();
    let mut named_recipes: Vec<(String, Recipe)> = Vec::new();

    for line in lines {
        match line {
            RawLine::Required(required_names) => {
                for name in required_names {
                    let _: bool = required.insert(resolve(&name));
                }
            }
            RawLine::Recipe(recipe) => {
                let inputs = recipe
                    .inputs
                    .into_iter()
                    .map(|term| {
                        Slot::new(BTreeSet::from([resolve(&term.name)]), None, term.count, 1.0)
                    })
                    .collect();
                let outputs = recipe
                    .outputs
                    .into_iter()
                    .map(|term| Output::new(resolve(&term.name), term.count, 1.0))
                    .collect();
                let id = recipe.name.clone();
                named_recipes.push((id, Recipe::new(recipe.name, 0, inputs, outputs, None, None)));
            }
        }
    }
    named_recipes.sort_by(|(a, _), (b, _)| a.cmp(b));
    let recipes = named_recipes
        .into_iter()
        .map(|(_, recipe)| recipe)
        .collect();

    let items = ids
        .into_iter()
        .map(|id| Item::new(id, ItemKind::Item, BTreeSet::new()))
        .collect();

    (
        Graph::new(1, items, recipes, Vec::new(), vec!["test".to_owned()]),
        required,
    )
}

#[cfg(test)]
mod tests {
    use core::iter;
    use std::collections::BTreeSet;

    use super::parse;
    use crate::model::ItemIndex;

    #[test]
    fn parses_the_stone_dust_example() {
        let (graph, required) = parse(
            "
            required concrete
            mix:    stone_dust + water -> concrete
            pack:   4 small_stone_dust -> stone_dust
            unpack: stone_dust -> 4 small_stone_dust
            ",
        );

        let json = serde_json::to_value(&graph).expect("serializing the parsed graph");
        let items: Vec<&str> = json
            .get("items")
            .and_then(serde_json::Value::as_array)
            .expect("items array")
            .iter()
            .map(|item| {
                item.get("id")
                    .and_then(serde_json::Value::as_str)
                    .expect("item id")
            })
            .collect();
        assert_eq!(
            items,
            vec!["concrete", "small_stone_dust", "stone_dust", "water"],
            "items should be sorted by id",
        );

        let recipes: Vec<&str> = json
            .get("recipes")
            .and_then(serde_json::Value::as_array)
            .expect("recipes array")
            .iter()
            .map(|recipe| {
                recipe
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .expect("recipe id")
            })
            .collect();
        assert_eq!(recipes, vec!["mix", "pack", "unpack"]);

        assert_eq!(required, iter::once(ItemIndex::new(0)).collect());
    }

    #[test]
    fn allows_an_empty_required_line_and_empty_input_and_output_sides() {
        let (graph, required) = parse(
            "
            required
            sink: iron_ingot ->
            source: -> gold_ingot
            ",
        );

        assert_eq!(required, BTreeSet::new());

        let json = serde_json::to_value(&graph).expect("serializing the parsed graph");
        let recipes = json
            .get("recipes")
            .and_then(serde_json::Value::as_array)
            .expect("recipes array");
        let slot_counts = |recipe_id: &str, field: &str| -> usize {
            recipes
                .iter()
                .find(|recipe| {
                    recipe.get("id").and_then(serde_json::Value::as_str) == Some(recipe_id)
                })
                .and_then(|recipe| recipe.get(field))
                .and_then(serde_json::Value::as_array)
                .expect("recipe field array")
                .len()
        };
        assert_eq!(slot_counts("sink", "inputs"), 1);
        assert_eq!(slot_counts("sink", "outputs"), 0);
        assert_eq!(slot_counts("source", "inputs"), 0);
        assert_eq!(slot_counts("source", "outputs"), 1);
    }

    #[test]
    #[should_panic(expected = "expected a count")]
    fn rejects_a_malformed_count() {
        drop(parse("mix: stone dust -> concrete"));
    }

    #[test]
    #[should_panic(expected = "missing '->'")]
    fn rejects_a_recipe_without_an_arrow() {
        drop(parse("mix: stone_dust + water"));
    }
}
