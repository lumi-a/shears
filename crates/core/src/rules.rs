//! Rules set local analysis inputs from `rules.toml`.
//!
//! Ordered selector -> action pairs: required items, propagation, stage and
//! decomposition. The last rule to match wins per attribute. Every rule must carry a
//! reason, for the report's sake.
//!
//! Every rule is either `[[item-rule]]` or `[[recipe-rule]]`. This header fixes which
//! selector keys and which actions are valid for that rule.
//!
//! TODO: Document the possible filters and actions here? Or perhaps include a link to the
//! documentation of both, which surely explain both things in detail.
//!
//! ```toml
//! [[item-rule]]
//! reason = "batteries are useful as inventory items and should be required craftable"
//! filter = { id = "/_battery$/" }
//! action = { required = true }
//!
//! [[item-rule]]
//! reason = "raw ores are obtainable from the world"
//! filter = { tag = "forge:raw_materials" }
//! action = { world_source = true }
//!
//! [[recipe-rule]]
//! reason = "recycling shouldn't keep anything else transitively required"
//! filter = { type = "gtceu:macerator" }
//! action = { propagate = "drop" }
//! ```
//!
//! TODO: Describe the above examples better.

pub mod selector;

use selector::{Context, Selector, SelectorError};
use toml::de::Error as TomlError;

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
    /// Sets every output edge's propagation state.
    ///
    /// TODO: At some point, we'll probably want to be able to match individual (recipe,
    /// output) tuples, instead of just recipes?
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
    /// # Panics
    ///
    /// If `recipe` has `u8::MAX` or more outputs.
    fn apply_to(&self, graph: &Graph, edges: &mut EdgeStates, recipe: RecipeIndex, cause: &Cause) {
        match *self {
            Self::Propagate(state) => {
                let output_count = graph.recipe(recipe).outputs().len();
                for output_position in 0..output_count {
                    let output = u8::try_from(output_position)
                        .expect("fewer than u8::MAX outputs per recipe");
                    edges.set(recipe, output, state, cause.clone());
                }
            }
        }
    }
}

/// One `[[item-rule]]` or `[[recipe-rule]]` entry, consisting of a selector, the action
/// it sets on every match, and why it does so.
///
/// TODO: Should we use a sum type instead of being generic on `<A>`?
#[derive(Debug)]
struct Rule<A> {
    /// The reason behind this rule.
    #[expect(dead_code, reason = "read by the explain.rs report eventually™")]
    reason: String,
    /// What this rule applies to.
    selector: Selector,
    /// What it sets on every match.
    action: A,
}

// TODO: The performance of for_each_matching_item and for_each_matching_recipe aren't
// particularly great. For one thing, it may be better to, instead of doing `for each
// rule: for each item;` instead do `for each item: for each rule;`. What would be more
// cache efficient? Perhaps a benchmark would help here. Furthermore, it would be nice if
// there were ways of setting these things without iterating over the set of all items /
// recipes. But that kinda sounds like I want to reinvent SQL. Do I _want_ to use SQL
// here? Are there some rust data types / crates that don't come with all the overhead of
// SQL but do much of the same thing?
// Perhaps one of the two variants lends itself better to parallelization / SIMD?

/// Calls `visit` with every item in `graph` that `selector` matches.
fn for_each_matching_item(graph: &Graph, selector: &Selector, mut visit: impl FnMut(ItemIndex)) {
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

/// A `rules.toml` document's `[[item-rule]]` and `[[recipe-rule]]` entries, parsed and
/// validated.
///
/// See the [module docs](self) for example rule files.
#[derive(Debug, Default)]
pub struct RuleSet {
    /// Every `[[item-rule]]` entry, in file order.
    item_rules: Vec<Rule<ItemAction>>,
    /// Every `[[recipe-rule]]` entry, in file order.
    recipe_rules: Vec<Rule<RecipeAction>>,
}

impl RuleSet {
    /// Parses one rules file's `[[item-rule]]` and `[[recipe-rule]]` entries.
    ///
    /// Either array may be omitted, meaning no rules of that kind.
    ///
    /// # Errors
    ///
    /// If the document isn't valid TOML, a rule is missing `reason`, `filter` or
    /// `action`, `action` doesn't contain *exactly* one action recognized for that rule's
    /// kind, or the selector uses a key invalid for that kind (see the [module
    /// docs](self) for what's in scope).
    #[inline]
    pub fn parse(source: &str) -> Result<Self, RuleError> {
        let document: toml::Value = toml::from_str(source)?;
        let root_table = document.as_table().ok_or(RuleError::WrongShape {
            key: "<root>",
            expected: "a table",
        })?;

        let item_rules = parse_rule_array(root_table, "item-rule", parse_item_rule)?;
        let recipe_rules = parse_rule_array(root_table, "recipe-rule", parse_recipe_rule)?;

        Ok(Self {
            item_rules,
            recipe_rules,
        })
    }

    /// Applies every rule in order.
    ///
    /// Last match winning per attribute, since later writes to
    /// [`NodeAttrs`]/[`EdgeStates`] simply overwrite earlier ones.
    ///
    /// Item rules and recipe rules never change the same attribute (TODO: ensure this
    /// remains the case, somehow?), so applying every item rule before every recipe rule
    /// doesn't change anything about the outcome (TODO: Will we even need this guarantee
    /// in the future? If not, we should drop it for now.).
    ///
    /// `file` identifies this rule file for [`Cause::Rule`]'s `file` field, in load order
    /// (presets first, then local).
    ///
    /// # Panics
    ///
    /// If this `RuleSet` has `u32::MAX` or more item rules or recipe rules, or a recipe
    /// has `u8::MAX` or more outputs.
    #[inline]
    pub fn apply(&self, graph: &Graph, attrs: &mut NodeAttrs, edges: &mut EdgeStates, file: u16) {
        for (position, rule) in self.item_rules.iter().enumerate() {
            let index = u32::try_from(position).expect("fewer than u32::MAX rules in one file");
            let cause = Cause::Rule {
                file,
                kind: RuleKind::Item,
                index,
            };
            for_each_matching_item(graph, &rule.selector, |item| {
                rule.action.apply_to(attrs, item, &cause);
            });
        }
        for (position, rule) in self.recipe_rules.iter().enumerate() {
            let index = u32::try_from(position).expect("fewer than u32::MAX rules in one file");
            let cause = Cause::Rule {
                file,
                kind: RuleKind::Recipe,
                index,
            };
            for_each_matching_recipe(graph, &rule.selector, |recipe| {
                rule.action.apply_to(graph, edges, recipe, &cause);
            });
        }
    }
}

/// Parses one top-level array of rule tables (`[[item-rule]]` or `[[recipe-rule]]`),
/// defaulting to empty if `key` is absent.
fn parse_rule_array<A>(
    root_table: &toml::Table,
    key: &'static str,
    parse_one: impl Fn(&toml::Value) -> Result<Rule<A>, RuleError>,
) -> Result<Vec<Rule<A>>, RuleError> {
    let Some(rules_value) = root_table.get(key) else {
        return Ok(Vec::new());
    };
    let entries = rules_value.as_array().ok_or(RuleError::WrongShape {
        key,
        expected: "an array of tables",
    })?;

    entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            parse_one(entry).map_err(|cause| RuleError::InRule {
                array: key,
                position,
                source: Box::new(cause),
            })
        })
        .collect()
}

/// Parses one `[[item-rule]]` table.
fn parse_item_rule(entry: &toml::Value) -> Result<Rule<ItemAction>, RuleError> {
    let (reason, action_table, filter_value) = parse_rule_fields(entry)?;
    let action = parse_item_action(action_table)?;
    let selector = selector::parse(filter_value, Context::Item)?;
    Ok(Rule {
        reason,
        selector,
        action,
    })
}

/// Parses one `[[recipe-rule]]` table.
fn parse_recipe_rule(entry: &toml::Value) -> Result<Rule<RecipeAction>, RuleError> {
    let (reason, action_table, filter_value) = parse_rule_fields(entry)?;
    let action = parse_recipe_action(action_table)?;
    let selector = selector::parse(filter_value, Context::Recipe)?;
    Ok(Rule {
        reason,
        selector,
        action,
    })
}

/// Parses the fields common to both rule kinds: `reason`, `action` (still a raw table)
/// and `filter` (still a raw value).
fn parse_rule_fields(
    entry: &toml::Value,
) -> Result<(String, &toml::Table, &toml::Value), RuleError> {
    let table = entry.as_table().ok_or(RuleError::WrongShape {
        key: "rule",
        expected: "a table",
    })?;

    let reason = table
        .get("reason")
        .and_then(toml::Value::as_str)
        .ok_or(RuleError::MissingField { field: "reason" })?
        .to_owned();

    let action_table = table
        .get("action")
        .and_then(toml::Value::as_table)
        .ok_or(RuleError::MissingField { field: "action" })?;

    let filter_value = table
        .get("filter")
        .ok_or(RuleError::MissingField { field: "filter" })?;

    Ok((reason, action_table, filter_value))
}

/// Parses an `[[item-rule]]`'s `action` table into exactly one [`ItemAction`].
fn parse_item_action(action_table: &toml::Table) -> Result<ItemAction, RuleError> {
    // TODO: It would be nice to isolate this kind of parsing to be more modular, so that
    // we don't repeat the string "world_source" everywhere in this function.
    const KNOWN: [&str; 2] = ["required", "world_source"];

    if let Some(unknown) = action_table
        .keys()
        .find(|key| !KNOWN.contains(&key.as_str()))
    {
        return Err(RuleError::UnknownAction(unknown.clone()));
    }

    let mut actions = Vec::new();
    if let Some(value) = action_table.get("required") {
        actions.push(ItemAction::Required(value.as_bool().ok_or(
            RuleError::WrongShape {
                key: "required",
                expected: "a bool",
            },
        )?));
    }
    if let Some(value) = action_table.get("world_source") {
        actions.push(ItemAction::WorldSource(value.as_bool().ok_or(
            RuleError::WrongShape {
                key: "world_source",
                expected: "a bool",
            },
        )?));
    }

    // TODO: I'm really not sure anymore if an empty action or multiple actions should
    // even be frowned upon. I mean empty actions should at least raise a warning i guess,
    // but they shouldn't just lead to errors. And as remarked on previously,
    // multiple actions in one rule may just be fine.
    match actions.len() {
        1 => Ok(actions.remove(0)),
        0 => Err(RuleError::EmptyAction),
        _ => Err(RuleError::MultipleActions),
    }
}

/// Parses a `[[recipe-rule]]`'s `action` table into exactly one [`RecipeAction`].
fn parse_recipe_action(action_table: &toml::Table) -> Result<RecipeAction, RuleError> {
    const KNOWN: [&str; 1] = ["propagate"];

    if let Some(unknown) = action_table
        .keys()
        .find(|key| !KNOWN.contains(&key.as_str()))
    {
        return Err(RuleError::UnknownAction(unknown.clone()));
    }

    let mut actions = Vec::new();
    if let Some(value) = action_table.get("propagate") {
        actions.push(RecipeAction::Propagate(parse_propagate(value)?));
    }

    match actions.len() {
        1 => Ok(actions.remove(0)),
        0 => Err(RuleError::EmptyAction),
        _ => Err(RuleError::MultipleActions),
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
    /// The document isn't valid TOML at all.
    #[error("invalid TOML: {0}")]
    Toml(#[from] TomlError),
    /// A value had the wrong TOML shape for its key.
    #[error("`{key}` should be {expected}")]
    WrongShape {
        /// The key whose value was malformed.
        key: &'static str,
        /// What shape was expected instead.
        expected: &'static str,
    },
    /// A rule, or the document itself, is missing a required field.
    #[error("missing `{field}`")]
    MissingField {
        /// The field that's missing.
        field: &'static str,
    },
    /// An `action` table had a key that isn't a recognized action.
    #[error("unknown action `{0}` in `action`")]
    UnknownAction(String),
    /// An `action` table had none of the recognized actions.
    #[error("`action` has no recognized action")]
    EmptyAction,
    /// An `action` table had more than one recognized action.
    ///
    /// For now, rules set exactly one, as I haven't yet seen a reasonable pair of rules
    /// that would often go together. If this ever happens, we can either talk about
    /// allowing multiple actions, or about creating a new action that does both things
    /// simultaneously. Until then, it's easier to just do it this way.
    ///
    /// TODO: Although... Perhaps it would be more natural for an end user. Why would we
    /// give them a table if they can't add multiple entries to that table?
    #[error("`action` has more than one action")]
    MultipleActions,
    /// The selector itself was invalid.
    #[error(transparent)]
    Selector(#[from] SelectorError),
    /// An error inside one `[[item-rule]]` or `[[recipe-rule]]` entry.
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
        assert!(matches!(
            error,
            RuleError::InRule { array: "item-rule", position: 0, source }
                if matches!(*source, RuleError::MissingField { field: "reason" })
        ));
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
        assert!(matches!(
            error,
            RuleError::InRule { array: "item-rule", position: 0, source }
                if matches!(&*source, RuleError::UnknownAction(key) if key == "propagate")
        ));
    }

    #[test]
    fn two_item_actions_in_one_table_is_an_error() {
        let result = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "nonsense"
            filter = { id = "x" }
            action = { required = true, world_source = true }
            "#,
        );

        let error = result.expect_err("required and world_source together is more than one action");
        assert!(matches!(
            error,
            RuleError::InRule { array: "item-rule", position: 0, source }
                if matches!(*source, RuleError::MultipleActions)
        ));
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
}
