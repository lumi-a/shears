//! Rules set local analysis inputs from `rules.toml`.
//!
//! Ordered selector -> action pairs. A selector selects a set of items or recipes, and an
//! action does things like "set the item to be required", "specify the item as a world
//! source", or "specify the recipe as useless".
//!
//! The **last** rule to match wins, for each attribute and item. Every rule must also carry a
//! reason so that the report is easier to read.
//!
//! ```toml
//! [[item-rule]]
//! reason = "oak boats are useful for travelling"
//! filter = { id = "minecraft:oak_boat" }
//! action = { required = true }
//! ```
//!
//! This selects the item with id `minecraft:oak_boat` and sets it as a required item. The `id`
//! field also supports regexes, if surrounded by slashes:
//!
//! ```toml
//! [[item-rule]]
//! reason = "batteries are useful as inventory items and in battery buffers"
//! filter = { mod = "gtceu", id = "/_battery$/" }
//! action = { required = true }
//! ```
//!
//! This finds all items in the `gtceu` (GregTech) namespace whose `id` ends with `_battery`
//! and sets them as required. `mod` and `id` are two selectors here: One filtering for mods,
//! one filtering for an `id`. The two conditions are ANDed together, because they are in
//! {curly braces} (a TOML inline table). In other words, they must both be true. If you would
//! like to instead have ORed selectors, i.e. two selectors for which only one needs to be
//! true, you can use \[square brackets\] (a TOML array) instead:
//!
//! ```toml
//! [[item-rule]]
//! reason = "ores can be found underground"
//! filter = [ { tag = "forge:raw_materials" }, { tag = "forge:ores" }]
//! action = { world_source = true }
//! ```
//!
//! This finds all items that have tag `forge:raw_materials` or `forge:ores` and specifies they
//! are all obtainable by just mining blocks in-world.
//!
//! Similarly, we can perform actions on individual recipes. Every rule is either
//! `[[item-rule]]` or `[[recipe-rule]]`. Depending on whether you're writing a recipe-rule or
//! item-rule, different selectors and actions are available, and you will be warned about
//! invalid ones. Here is a recipe rule:
//!
//! ```toml
//! [[recipe-rule]]
//! reason = "recycling pipes shouldn't ever be counted as a material source"
//! filter = { type = "gtceu:macerator", id = "/macerate_.*_fluid_pipe$/" }
//! action = { propagate = "drop" }
//! ```
//!
//! TODO: This example would be a lot better had we already implemented recipe categories (not
//! just types). Also, do recipe types correlate exactly to their path tail?
//!
//! This finds all GregTech recipes that macerate a fluid pipe and sets their propagation state
//! to "drop", which translates to something like "When asked how to obtain copper dust, please
//! don't suggest macerating copper pipes as a useful source". If a recipe has multiple
//! outputs, you may want to say that the recipe is a useful source of one of the outputs, but
//! not a useful source for a different output:
//!
//! ```toml
//! [[recipe-rule]]
//! reason = "stone dust from ore grinding is junk"
//! filter = { type = "gtceu:macerator", id = "/_ore_/" }
//! apply_to_output = { id = "gtceu:stone_dust" }
//! action = { propagate = "drop" }
//! ```
//!
//! For instance, consider the GregTech recipe that macerates one gold ore into crushed gold
//! ore and some stone dust. This filter means that:
//! - When asking "What's a useful source of stone dust?", gold ore maceration will **not** be
//!   brought up, but
//! - When asking "What's a useful source of crushed gold ore?", gold ore maceration will be
//!   brought up.
//!
//! You can also filter recipes by what inputs / outputs they have. For instance:
//!
//! ```toml
//! [[recipe-rule]]
//! reason = "the greenhouse recipes using fertilizer are always stronger"
//! filter = { type = "gtceu:greenhouse", not = { has_input = { id = "gtceu:fertilizer" } } }
//! action = { propagate = "drop" }
//! ```
//!
//! This finds all greenhouse recipes that do not have `gtceu:fertilizer` as one of its inputs,
//! and marks them as useless for all outputs.
//!
//! ## Selectors
//!
//! The field `filter` is a [`Selector`], fully described there. In short:
//!
//! - `id`, `mod`, `type`: Item or recipe, matched exactly as a string, or against a `/regex/`
//!   if surrounded by slashes.
//! - `tag`: Item only, matches tag membership.
//! - `has_input`, `has_output`: Recipe only, matches if any of the recipe's input/output slots
//!   match the contained selector.
//! - `is_catalyst`: Only valid inside a `has_input` selector, matches a slot whose chance is
//!   zero, i.e. it's never actually consumed or produced.
//! - A TOML {table} ANDs its keys together. An \[array\] ORs its entries. A `not` negates.
//!
//! ## Actions
//!
//! `[[item-rule]]`'s `action` may set any number of these:
//! - `required` (bool, default false): marks or unmarks the item as directly required.
//! - `world_source` (bool, default false): marks or unmarks the item as obtainable from the
//!   world, meaning it needs no recipe that produces it, because the player can just mine it
//!   from the world.
//!
//! `[[recipe-rule]]`'s `action` may set:
//!
//! - `propagate` (`true`, `"inert"`, or `"drop"`, default `true`): Consider a recipe, and
//!   consider all its possible output items, with an arrow going from each output item to the
//!   recipe. An arrow from an item to a recipe signifies that the recipe is a useful way of
//!   obtaining that item. "drop" drops the arrow. Unless specified otherwise, this hides _all_
//!   the arrows of the recipe. If you wish to only hide certain arrows, the additional field
//!   `apply_to_output` only hides the arrows that match the selector in `apply_to_output`. The
//!   three values `true`, `"inert"` and `"drop"` correspond to [`EdgeState::Propagate`],
//!   [`Inert`](EdgeState::Inert), and [`Dropped`](EdgeState::Dropped).

pub mod selector;

/// Every fenced code block tagged `` ```<language> `` in `source`'s doc comment, with the
/// comment prefix and the fence markers stripped.
#[cfg(test)]
#[expect(
    clippy::shadow_reuse,
    reason = "stripping an outer then an inner prefix off the same line"
)]
fn doc_comment_fenced_blocks(source: &str, language: &str) -> Vec<String> {
    let fence_start = format!("```{language}");
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in source.lines() {
        let Some(after_bang) = line.strip_prefix("//!") else {
            continue;
        };
        let after_bang = after_bang.strip_prefix(' ').unwrap_or(after_bang);
        if let Some(block) = current.as_mut() {
            if after_bang == "```" {
                blocks.push(current.take().expect("just matched Some above"));
            } else {
                block.push_str(after_bang);
                block.push('\n');
            }
        } else if after_bang == fence_start {
            current = Some(String::new());
        } else {
            // A line outside any fenced block, not our business.
        }
    }
    assert!(
        current.is_none(),
        "an unterminated ```{language} block in the module doc comment"
    );
    blocks
}

use serde::Deserialize;
use toml::de::Error as TomlError;

use selector::{Context, Selector, SelectorError};

use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs, RuleKind};
use crate::model::{Graph, ItemIndex, RecipeIndex};

/// What one `[[item-rule]]` does to every item its selector matches.
#[derive(Debug)]
enum ItemAction {
    /// Marks or unmarks an item as required directly.
    Required(bool),
    /// Marks or unmarks a world source.
    WorldSource(bool),
}

/// What one `[[recipe-rule]]` does to every recipe its selector matches.
#[derive(Debug)]
enum RecipeAction {
    /// Sets every matching output edge's propagation state.
    ///
    /// See `apply_to_output` on `[[recipe-rule]]` to narrow which outputs match, instead
    /// of matching all of them.
    Propagate(EdgeState),
}

impl ItemAction {
    /// Applies this action to one matching item.
    fn apply_to(&self, attrs: &mut NodeAttrs, item: ItemIndex, cause: &Cause) {
        match *self {
            Self::Required(true) => attrs.set_required(item, cause.clone()),
            Self::Required(false) => attrs.clear_required(item),
            Self::WorldSource(true) => attrs.set_world_source(item),
            Self::WorldSource(false) => attrs.clear_world_source(item),
        }
    }
}

impl RecipeAction {
    /// Applies this action to one matching recipe.
    ///
    /// `output_filter` (if set) narrows this to only the recipe's outputs matching it.
    ///
    /// # Panics
    ///
    /// If `recipe` has `u8::MAX` or more outputs.
    fn apply_to(
        &self,
        graph: &Graph,
        edges: &mut EdgeStates,
        recipe: RecipeIndex,
        output_filter: Option<&Selector>,
        cause: &Cause,
    ) {
        match *self {
            Self::Propagate(state) => {
                for (output_position, output) in
                    graph.recipe(recipe).outputs().iter().enumerate()
                {
                    if output_filter.is_some_and(|filter| {
                        !selector::matches_item(filter, graph, output.item())
                    }) {
                        continue;
                    }
                    let output_index = u8::try_from(output_position)
                        .expect("fewer than u8::MAX outputs per recipe");
                    edges.set(recipe, output_index, state, cause.clone());
                }
            }
        }
    }
}

/// An `[[item-rule]]`'s `action` table.
///
/// Every recognized key is optional and any number may be set at once.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItemAction {
    /// Marks or unmarks matching items as directly required, if set.
    required: Option<bool>,
    /// Marks or unmarks matching items as a world source, if set.
    world_source: Option<bool>,
}

impl RawItemAction {
    /// Turns the fields that were set into [`ItemAction`]s, in a fixed order so applying
    /// them is deterministic.
    fn into_actions(self) -> Vec<ItemAction> {
        let mut actions = Vec::new();
        if let Some(value) = self.required {
            actions.push(ItemAction::Required(value));
        }
        if let Some(value) = self.world_source {
            actions.push(ItemAction::WorldSource(value));
        }
        actions
    }
}

/// A `[[recipe-rule]]`'s `action` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecipeAction {
    /// Sets every output edge's propagation state, if the `Option` isn't `None`.
    propagate: Option<toml::Value>,
}

impl RawRecipeAction {
    /// Turns the fields that were set into [`RecipeAction`]s.
    ///
    /// # Errors
    ///
    /// If `propagate`'s value isn't `true`, `"inert"`, or `"drop"`.
    fn into_actions(self) -> Result<Vec<RecipeAction>, RuleError> {
        let mut actions = Vec::new();
        if let Some(value) = self.propagate {
            actions.push(RecipeAction::Propagate(parse_propagate(&value)?));
        }
        Ok(actions)
    }
}

/// One `[[item-rule]]` table. `filter` isn't resolved into a [`Selector`] yet and `action`
/// isn't turned into concrete actions yet.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItemRule {
    /// The reason behind this rule.
    reason: String,
    /// What this rule applies to. Still raw; not yet resolved into a [`Selector`].
    filter: toml::Value,
    /// What it sets on every match.
    action: RawItemAction,
}

/// One `[[recipe-rule]]` table, raw.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecipeRule {
    /// The reason behind this rule.
    reason: String,
    /// What this rule applies to. Still raw; not yet resolved into a [`Selector`].
    filter: toml::Value,
    /// Narrows the action to only the matched recipe's outputs matching this, instead
    /// of every output.
    apply_to_output: Option<toml::Value>,
    /// What it sets on every match.
    action: RawRecipeAction,
}

/// A `rules.toml` document's `[[item-rule]]` and `[[recipe-rule]]` entries.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDocument {
    /// Every `[[item-rule]]` entry, in file order.
    #[serde(rename = "item-rule", default)]
    item_rule: Vec<RawItemRule>,
    /// Every `[[recipe-rule]]` entry, in file order.
    #[serde(rename = "recipe-rule", default)]
    recipe_rule: Vec<RawRecipeRule>,
}

/// One `[[item-rule]]` entry, consisting of a selector, every action it sets on each match,
/// and why it does so.
#[derive(Debug)]
struct ItemRule {
    /// The reason behind this rule.
    reason: String,
    /// What this rule applies to.
    selector: Selector,
    /// What it sets on every match, in file order. Empty is valid, though it does
    /// nothing and [`RuleSet::parse`] reports this case as a warning.
    actions: Vec<ItemAction>,
}

/// One `[[recipe-rule]]` entry.
#[derive(Debug)]
struct RecipeRule {
    /// The reason behind this rule.
    reason: String,
    /// What this rule applies to.
    selector: Selector,
    /// Narrows the action to only the matched recipe's outputs matching this, instead
    /// of every output. `None` means every output.
    output_filter: Option<Selector>,
    /// What it sets on every match, in file order. Empty is valid, though it does
    /// nothing and [`RuleSet::parse`] reports this case as a warning.
    actions: Vec<RecipeAction>,
}

// TODO: The performance of for_each_matching_item and for_each_matching_recipe aren't
// particularly great. For one thing, it may be better to, instead of doing `for each
// rule: for each item;` instead do `for each item: for each rule;`. What would be more
// cache efficient? Perhaps a benchmark would help here. Furthermore, it would be nice if
// there were ways of setting these things without iterating over the set of all items /
// recipes. But that kinda sounds like I want to reinvent SQL. Do I _want_ to use SQL
// here? Are there some rust data types / crates that don't come with all the overhead of
// SQL but do much of the same thing?Perhaps one of the two variants lends itself better
// to parallelization / SIMD?

/// Calls `visit` with every item in `graph` that `selector` matches.
fn for_each_matching_item(
    graph: &Graph,
    selector: &Selector,
    mut visit: impl FnMut(ItemIndex),
) {
    for position in 0..graph.items().len() {
        let item = ItemIndex::new(u32::try_from(position).expect("fewer than u32::MAX items"));
        if selector::matches_item(selector, graph, item) {
            visit(item);
        }
    }
}

/// Calls `visit` with every recipe in `graph` that `selector` matches.
fn for_each_matching_recipe(
    graph: &Graph,
    selector: &Selector,
    mut visit: impl FnMut(RecipeIndex),
) {
    for position in 0..graph.recipes().len() {
        let recipe =
            RecipeIndex::new(u32::try_from(position).expect("fewer than u32::MAX recipes"));
        if selector::matches_recipe(selector, graph, recipe) {
            visit(recipe);
        }
    }
}

/// A `rules.toml` document's `[[item-rule]]` and `[[recipe-rule]]` entries, already parsed and
/// validated.
///
/// See the [module docs](self) for example rule files.
#[derive(Debug, Default)]
pub struct RuleSet {
    /// Every `[[item-rule]]` entry, in file order.
    item_rules: Vec<ItemRule>,
    /// Every `[[recipe-rule]]` entry, in file order.
    recipe_rules: Vec<RecipeRule>,
    /// Human-readable warnings about things that parsed fine but are probably mistakes,
    /// e.g. an `action` table that sets nothing.
    ///
    /// TODO: Should there be more warnings? E.g. about filters that didn't match against
    /// anything, item names that don't exist, chance values outside of \[0,1\], or do
    /// these go elsewhere? Should this be some bespoke enum instead of a String?
    warnings: Vec<String>,
}

impl RuleSet {
    /// Parses one rules file's `[[item-rule]]` and `[[recipe-rule]]` entries.
    ///
    /// Either array may be omitted, meaning no rules of that kind. An `action` table
    /// may set any number of its rule kind's actions, including zero. An empty `action`
    /// does nothing, but reports to [`Self::warnings`].
    ///
    /// # Errors
    ///
    /// If the document's shape is wrong, e.g. a missing `reason`/`filter`/`action`, an
    /// unrecognized key, or a value of the wrong TOML type.
    ///
    /// The error comes from [`serde_path_to_error`] and names the exact path to the
    /// problem, e.g. `item-rule[3].action: unknown field` followed by the offending
    /// key's name. Also errors if a `propagate` value isn't `true`, `"inert"`, or
    /// `"drop"`, or if a selector uses a key invalid for its rule kind.
    #[inline]
    pub fn parse(source: &str) -> Result<Self, RuleError> {
        let deserializer = toml::Deserializer::new(source);
        let document: RawDocument = serde_path_to_error::deserialize(deserializer)?;

        let mut warnings = Vec::new();

        let item_rules = document
            .item_rule
            .into_iter()
            .enumerate()
            .map(|(position, raw)| {
                let actions = raw.action.into_actions();
                if actions.is_empty() {
                    warnings.push(format!(
                        "item-rule[{position}]'s action table is empty, this rule does nothing"
                    ));
                }
                let selector =
                    selector::parse(&raw.filter, Context::Item).map_err(|cause| {
                        RuleError::InRule {
                            array: "item-rule",
                            position,
                            source: Box::new(RuleError::Selector(cause)),
                        }
                    })?;
                Ok(ItemRule {
                    reason: raw.reason,
                    selector,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, RuleError>>()?;

        let recipe_rules = document
            .recipe_rule
            .into_iter()
            .enumerate()
            .map(|(position, raw)| {
                let actions = raw
                    .action
                    .into_actions()
                    .map_err(|cause| RuleError::InRule {
                        array: "recipe-rule",
                        position,
                        source: Box::new(cause),
                    })?;
                if actions.is_empty() {
                    warnings.push(format!(
                        "recipe-rule[{position}]'s action table is empty, this rule does nothing"
                    ));
                }
                let selector =
                    selector::parse(&raw.filter, Context::Recipe).map_err(|cause| {
                        RuleError::InRule {
                            array: "recipe-rule",
                            position,
                            source: Box::new(RuleError::Selector(cause)),
                        }
                    })?;
                let output_filter = raw
                    .apply_to_output
                    .map(|value| selector::parse(&value, Context::Item))
                    .transpose()
                    .map_err(|cause| RuleError::InRule {
                        array: "recipe-rule",
                        position,
                        source: Box::new(RuleError::Selector(cause)),
                    })?;
                Ok(RecipeRule {
                    reason: raw.reason,
                    selector,
                    output_filter,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, RuleError>>()?;

        Ok(Self {
            item_rules,
            recipe_rules,
            warnings,
        })
    }

    /// Human-readable warnings about things that parsed fine but are probably mistakes.
    #[inline]
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The reason behind the `index`-th rule of kind `kind`, if that rule exists in this
    /// set. For `explain`'s sake, to turn a [`Cause::Rule`] back into report text.
    #[inline]
    #[must_use]
    pub fn reason(&self, kind: RuleKind, index: u32) -> Option<&str> {
        let position = usize::try_from(index).ok()?;
        match kind {
            RuleKind::Item => self
                .item_rules
                .get(position)
                .map(|rule| rule.reason.as_str()),
            RuleKind::Recipe => self
                .recipe_rules
                .get(position)
                .map(|rule| rule.reason.as_str()),
        }
    }

    /// Applies every rule in order.
    ///
    /// Last match winning per attribute, since later writes to
    /// [`NodeAttrs`]/[`EdgeStates`] simply overwrite earlier ones.
    ///
    /// Item rules and recipe rules never change the same attribute, so applying every item
    /// rule before every recipe rule doesn't change anything about the outcome.
    ///
    /// `file` identifies this rule file for [`Cause::Rule`]'s `file` field, in load order
    /// (presets first, then local).
    ///
    /// # Panics
    ///
    /// If this `RuleSet` has `u32::MAX` or more item rules or recipe rules, or a recipe
    /// has `u8::MAX` or more outputs.
    #[inline]
    pub fn apply(
        &self,
        graph: &Graph,
        attrs: &mut NodeAttrs,
        edges: &mut EdgeStates,
        file: u16,
    ) {
        for (position, rule) in self.item_rules.iter().enumerate() {
            let index =
                u32::try_from(position).expect("fewer than u32::MAX rules in one file");
            let cause = Cause::Rule {
                file,
                kind: RuleKind::Item,
                index,
            };
            for_each_matching_item(graph, &rule.selector, |item| {
                for action in &rule.actions {
                    action.apply_to(attrs, item, &cause);
                }
            });
        }
        for (position, rule) in self.recipe_rules.iter().enumerate() {
            let index =
                u32::try_from(position).expect("fewer than u32::MAX rules in one file");
            let cause = Cause::Rule {
                file,
                kind: RuleKind::Recipe,
                index,
            };
            for_each_matching_recipe(graph, &rule.selector, |recipe| {
                for action in &rule.actions {
                    action.apply_to(graph, edges, recipe, rule.output_filter.as_ref(), &cause);
                }
            });
        }
    }
}

/// Parses a `propagate` action's value: `true`, `"inert"`, or `"drop"`.
fn parse_propagate(value: &toml::Value) -> Result<EdgeState, RuleError> {
    match value {
        toml::Value::Boolean(true) => Ok(EdgeState::Propagate),
        toml::Value::String(state) if state == "inert" => Ok(EdgeState::Inert),
        toml::Value::String(state) if state == "drop" => Ok(EdgeState::Dropped),
        toml::Value::String(_)
        | toml::Value::Integer(_)
        | toml::Value::Float(_)
        | toml::Value::Boolean(_)
        | toml::Value::Datetime(_)
        | toml::Value::Array(_)
        | toml::Value::Table(_) => Err(RuleError::WrongShape {
            key: "propagate",
            expected: "true, \"inert\", or \"drop\"",
        }),
    }
}

/// A problem parsing or validating a rules file.
#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    /// The document's shape was wrong.
    ///
    /// Could have been a missing field, an unrecognized key, or a value of the wrong TOML
    /// type.
    ///
    /// Comes from [`serde_path_to_error`], so its `Display` already names the exact
    /// path to the offending value, e.g. `item-rule[3].action: missing field` followed
    /// by the missing field's name.
    #[error(transparent)]
    Deserialize(#[from] serde_path_to_error::Error<TomlError>),
    /// A value had the wrong TOML shape for its key.
    #[error("`{key}` should be {expected}")]
    WrongShape {
        /// The key whose value was malformed.
        key: &'static str,
        /// What shape was expected instead.
        expected: &'static str,
    },
    /// The selector itself was invalid.
    #[error(transparent)]
    Selector(#[from] SelectorError),
    /// An error inside one `[[item-rule]]` or `[[recipe-rule]]` entry, found while
    /// validating its selector or its `propagate` value (after the document itself
    /// already deserialized successfully).
    #[error("{array}[{position}]: {source}")]
    InRule {
        /// Which array this rule came from: `"item-rule"` or `"recipe-rule"`.
        array: &'static str,
        /// The rule's position within that array.
        position: usize,
        /// What went wrong inside it.
        #[source]
        source: Box<Self>,
    },
}

#[cfg(test)]
mod tests {
    use super::selector::{Context, SelectorError};
    use super::{RuleError, RuleSet};
    use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs, RuleKind};
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    /// Get the toml source of all the docstring examples of this crate.
    fn module_doc_toml_examples() -> Vec<String> {
        super::doc_comment_fenced_blocks(include_str!("rules.rs"), "toml")
    }

    /// Validate all the toml docstring examples.
    #[test]
    #[expect(clippy::panic, reason = "this is only used in tests")]
    fn every_module_doc_toml_example_is_a_valid_rules_toml_document() {
        let examples = module_doc_toml_examples();
        assert!(
            !examples.is_empty(),
            "should there not be some ```toml examples in the module doc?"
        );
        for (index, example) in examples.iter().enumerate() {
            let _: RuleSet = RuleSet::parse(example).unwrap_or_else(|error| {
                panic!("doc example {index} failed to parse: {error}\n{example}")
            });
        }
    }

    #[test]
    fn required_action_marks_matching_items() {
        let (graph, _required) = parse("mix: stone_dust -> gtceu:battery");
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "batteries are useful"
            filter = { id = "/battery$/" }
            action = { required = true }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Sorted item order: gtceu:battery=0, stone_dust=1.
        assert!(attrs.is_required(ItemIndex::new(0)));
        assert!(!attrs.is_required(ItemIndex::new(1)));
        assert!(matches!(
            attrs.required_cause(ItemIndex::new(0)),
            Some(Cause::Rule {
                file: 0,
                kind: RuleKind::Item,
                index: 0
            })
        ));
    }

    #[test]
    fn required_false_unmarks_a_preset_required_item() {
        let (graph, _required) = parse("mix: stone_dust -> concrete");
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "don't want this one after all"
            filter = { id = "concrete" }
            action = { required = false }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        attrs.set_required(ItemIndex::new(0), Cause::Pseudo);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        assert!(!attrs.is_required(ItemIndex::new(0)));
    }

    #[test]
    fn world_source_action_marks_matching_items() {
        let (graph, _required) = parse("mix: minecraft:iron_ore -> minecraft:iron_ingot");
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "ores spawn in the world"
            filter = { id = "/_ore$/" }
            action = { world_source = true }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Sorted item order: minecraft:iron_ingot=0, minecraft:iron_ore=1.
        assert!(attrs.is_world_source(ItemIndex::new(1)));
        assert!(!attrs.is_world_source(ItemIndex::new(0)));
    }

    #[test]
    fn two_item_actions_in_one_table_both_apply() {
        let (graph, _required) = parse("mix: minecraft:iron_ore -> minecraft:iron_ingot");
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "ores are both required and world-sourced"
            filter = { id = "/_ore$/" }
            action = { required = true, world_source = true }
            "#,
        )
        .expect("an action table may set more than one action at once");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Sorted item order: minecraft:iron_ingot=0, minecraft:iron_ore=1.
        assert!(attrs.is_required(ItemIndex::new(1)));
        assert!(attrs.is_world_source(ItemIndex::new(1)));
    }

    #[test]
    fn an_empty_action_table_does_nothing_but_warns() {
        let (graph, _required) = parse("mix: stone_dust -> concrete");
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "oops, forgot to fill this in"
            filter = { id = "concrete" }
            action = {}
            "#,
        )
        .expect("an empty action table is valid, just useless");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        assert!(!attrs.is_required(ItemIndex::new(0)), "nothing was set");
        assert_eq!(rules.warnings().len(), 1);
        assert!(rules
            .warnings()
            .first()
            .expect("exactly one warning")
            .contains("item-rule[0]"));
    }

    #[test]
    fn propagate_action_sets_every_output_edge() {
        let (graph, _required) = parse("macerate: ore -> 2 dust + 1 dust");
        let rules = RuleSet::parse(
            r#"
            [[recipe-rule]]
            reason = "recycling, drop it"
            filter = { id = "macerate" }
            action = { propagate = "drop" }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        assert_eq!(edges.state(RecipeIndex::new(0), 0), EdgeState::Dropped);
        assert_eq!(edges.state(RecipeIndex::new(0), 1), EdgeState::Dropped);
    }

    #[test]
    fn apply_to_output_scopes_propagate_to_matching_outputs() {
        let (graph, _required) = parse("macerate: ore -> cobblestone + dust");
        let rules = RuleSet::parse(
            r#"
            [[recipe-rule]]
            reason = "cobblestone from the macerator is junk, but keep the ore dust"
            filter = { id = "macerate" }
            apply_to_output = { id = "cobblestone" }
            action = { propagate = "drop" }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Outputs keep the order they were written in: cobblestone=0, dust=1.
        assert_eq!(
            edges.state(RecipeIndex::new(0), 0),
            EdgeState::Dropped,
            "cobblestone matched apply_to_output"
        );
        assert_eq!(
            edges.state(RecipeIndex::new(0), 1),
            EdgeState::Propagate,
            "dust didn't match apply_to_output, so it's untouched"
        );
    }

    #[test]
    fn apply_to_output_on_item_rule_is_unknown() {
        let result = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "nonsense"
            filter = { id = "x" }
            apply_to_output = { id = "y" }
            action = { required = true }
            "#,
        );

        let error = result.expect_err("apply_to_output isn't a recognized item-rule field");
        let message = error.to_string();
        assert!(
            message.contains("item-rule[0]") && message.contains("apply_to_output"),
            "expected the entry's path and the offending key, got: {message}"
        );
    }

    #[test]
    fn later_rules_win_over_earlier_ones_for_the_same_item() {
        let (graph, _required) = parse("mix: stone_dust -> concrete");
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "preset: every dust is required"
            filter = { id = "/dust/" }
            action = { required = true }

            [[item-rule]]
            reason = "local: actually, not this one"
            filter = { id = "stone_dust" }
            action = { required = false }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Sorted item order: concrete=0, stone_dust=1.
        assert!(!attrs.is_required(ItemIndex::new(1)), "the later rule wins");
    }

    #[test]
    fn missing_reason_is_an_error() {
        let result = RuleSet::parse(
            r#"
            [[item-rule]]
            filter = { id = "x" }
            action = { required = true }
            "#,
        );

        let error = result.expect_err("rule 0 is missing `reason`");
        let message = error.to_string();
        assert!(
            message.contains("item-rule[0]") && message.contains("reason"),
            "expected the entry's path and the missing field's name, got: {message}"
        );
    }

    #[test]
    fn a_recipe_action_under_item_rule_is_unknown() {
        let result = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "nonsense"
            filter = { id = "x" }
            action = { propagate = "drop" }
            "#,
        );

        let error = result.expect_err("propagate isn't a recognized item-rule action");
        let message = error.to_string();
        assert!(
            message.contains("item-rule[0]") && message.contains("propagate"),
            "expected the entry's path and the offending key, got: {message}"
        );
    }

    #[test]
    fn a_selector_invalid_for_its_rule_kind_is_an_error() {
        let result = RuleSet::parse(
            r#"
            [[recipe-rule]]
            reason = "tag doesn't apply to a recipe-targeting rule"
            filter = { tag = "forge:tools" }
            action = { propagate = "drop" }
            "#,
        );

        let error = result.expect_err("tag doesn't apply to a Context::Recipe selector");
        assert!(matches!(
            error,
            RuleError::InRule { array: "recipe-rule", position: 0, source } if matches!(
                *source,
                RuleError::Selector(SelectorError::WrongContext {
                    key: "tag",
                    context: Context::Recipe,
                })
            )
        ));
    }

    #[test]
    fn both_rule_arrays_are_optional() {
        let (graph, _required) = parse("mix: stone_dust -> concrete");
        let rules = RuleSet::parse("").expect("a rules file with neither array is valid");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        assert!(
            !attrs.is_required(ItemIndex::new(0)),
            "nothing marked it required"
        );
    }

    #[test]
    fn reason_looks_up_by_kind_and_position() {
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "batteries are useful"
            filter = { id = "x" }
            action = { required = true }

            [[recipe-rule]]
            reason = "recycling shouldn't keep anything else required"
            filter = { id = "y" }
            action = { propagate = "drop" }
            "#,
        )
        .expect("valid rules");

        assert_eq!(
            rules.reason(RuleKind::Item, 0),
            Some("batteries are useful")
        );
        assert_eq!(
            rules.reason(RuleKind::Recipe, 0),
            Some("recycling shouldn't keep anything else required")
        );
        assert_eq!(rules.reason(RuleKind::Item, 1), None, "no such item-rule");
        assert_eq!(
            rules.reason(RuleKind::Recipe, 1),
            None,
            "no such recipe-rule"
        );
    }
}
