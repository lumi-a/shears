//! Rules set local analysis inputs from `rules.toml`.
//!
//! Ordered selector -> action pairs: wanted items, propagation, stage and decomposition.
//! The last rule to match wins per attribute. Every rule must carry a reason, for the
//! report's sake.
//!
//! Settled on "wanted" (opt-in/asserted directly) and "needed" (computed/transitive,
//! what `liveness.rs`'s `Needed` used to call `Liveness`) over "root"/"alive": in the
//! natural perception of the recipe-item graph (recipes point to their outputs, inputs
//! point to their recipes), the things we called "roots" are actually leaves, it's just
//! that the BFS runs in reverse -- "wanted"/"needed" sidestep that confusion entirely.
//!
//! ```toml
//! [[rule]]
//! reason = "batteries are useful as inventory items"
//! filter = { id = "/_battery$/" }
//! action = { wanted = true }
//!
//! [[rule]]
//! reason = "raw ores are obtainable from the world"
//! filter = { tag = "forge:raw_materials" }
//! action = { world_source = true }
//!
//! [[rule]]
//! reason = "recycling shouldn't keep anything else needed"
//! filter = { type = "gtceu:macerator" }
//! action = { propagate = "drop" }
//! ```
//!
//! TODO: Describe the above examples better.

pub mod selector;

use selector::{Context, Selector, SelectorError};
use toml::de::Error as TomlError;

use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
use crate::model::{Graph, ItemIndex, RecipeIndex};

/// What one rule does to the items or recipes its selector matches.
#[derive(Debug)]
enum Action {
    /// Acts on items; the selector must be a [`Context::Item`] selector.
    Item(ItemAction),
    /// Acts on recipes; the selector must be a [`Context::Recipe`] selector.
    Recipe(RecipeAction),
}

/// An [`Action`] that applies to every item its rule's selector matches.
#[derive(Debug)]
enum ItemAction {
    /// Marks or unmarks an item as wanted directly.
    Wanted(bool),
    /// Marks or unmarks a world source.
    WorldSource(bool),
}

/// An [`Action`] that applies to every recipe its rule's selector matches.
#[derive(Debug)]
enum RecipeAction {
    /// Sets every output edge's propagation state.
    ///
    /// TODO: At some point, we'll probably want to be able to match individual (recipe,
    /// output) tuples, instead of just recipes?
    Propagate(EdgeState),
}

impl Action {
    /// Which [`Context`] this action's selector must be validated against.
    const fn context(&self) -> Context {
        match self {
            Self::Item(_) => Context::Item,
            Self::Recipe(_) => Context::Recipe,
        }
    }
}

impl ItemAction {
    /// Applies this action to one matching item.
    fn apply_to(&self, attrs: &mut NodeAttrs, item: ItemIndex, cause: &Cause) {
        match *self {
            Self::Wanted(true) => attrs.set_wanted(item, cause.clone()),
            Self::Wanted(false) => attrs.clear_wanted(item),
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

/// One `[[rule]]` entry, consisting of a selector, the action it sets, and why.
#[derive(Debug)]
struct Rule {
    /// The reason behind this rule.
    #[expect(dead_code, reason = "read by the explain.rs report eventually™")]
    reason: String,
    /// What this rule applies to.
    selector: Selector,
    /// What it sets on every match.
    action: Action,
}

impl Rule {
    /// Applies this rule to every matching item or recipe.
    ///
    /// # Panics
    ///
    /// If `graph` has `u32::MAX` or more items or recipes, or a recipe has `u8::MAX` or
    /// more outputs.
    fn apply_to(
        &self,
        graph: &Graph,
        attrs: &mut NodeAttrs,
        edges: &mut EdgeStates,
        cause: &Cause,
    ) {
        match &self.action {
            Action::Item(item_action) => {
                for_each_matching_item(graph, &self.selector, |item| {
                    item_action.apply_to(attrs, item, cause);
                });
            }
            Action::Recipe(recipe_action) => {
                for_each_matching_recipe(graph, &self.selector, |recipe| {
                    recipe_action.apply_to(graph, edges, recipe, cause);
                });
            }
        }
    }
}

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

/// A `rules.toml` document's `[[rule]]` entries, already parsed and validated.
///
/// See the [module docs](self) for example rule files.
#[derive(Debug, Default)]
pub struct RuleSet {
    /// Every rule, in file order.
    rules: Vec<Rule>,
}

impl RuleSet {
    /// Parses one rules file's `[[rule]]` entries.
    ///
    /// # Errors
    ///
    /// If the document isn't valid TOML, a rule is missing `reason`, `filter` or
    /// `action`, `action` doesn't contain exactly one recognized action, or the selector
    /// is invalid for that action's context (see the [module docs](self) for what's in
    /// scope).
    #[inline]
    pub fn parse(source: &str) -> Result<Self, RuleError> {
        let document: toml::Value = toml::from_str(source)?;
        let root_table = document.as_table().ok_or(RuleError::WrongShape {
            key: "<root>",
            expected: "a table",
        })?;
        let rules_value = root_table
            .get("rule")
            .ok_or(RuleError::MissingField { field: "rule" })?;
        let entries = rules_value.as_array().ok_or(RuleError::WrongShape {
            key: "rule",
            expected: "an array of tables",
        })?;

        let rules = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| {
                parse_rule(entry).map_err(|cause| RuleError::InRule {
                    position,
                    source: Box::new(cause),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { rules })
    }

    /// Applies every rule in order.
    ///
    /// Last match winning per attribute, since later writes to
    /// [`NodeAttrs`]/[`EdgeStates`] simply overwrite earlier ones.
    ///
    /// `file` identifies this rule file for [`Cause::Rule`]'s `file` field, in load order
    /// (presets first, then local); the caller assigns it.
    ///
    /// # Panics
    ///
    /// If this `RuleSet` has `u32::MAX` or more rules, or applying a rule would (see
    /// `Rule::apply_to`).
    #[inline]
    pub fn apply(&self, graph: &Graph, attrs: &mut NodeAttrs, edges: &mut EdgeStates, file: u16) {
        for (position, rule) in self.rules.iter().enumerate() {
            let index = u32::try_from(position).expect("fewer than u32::MAX rules in one file");
            let cause = Cause::Rule { file, index };
            rule.apply_to(graph, attrs, edges, &cause);
        }
    }
}

/// Parses one `[[rule]]` table.
fn parse_rule(entry: &toml::Value) -> Result<Rule, RuleError> {
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
    let action = parse_action(action_table)?;

    let filter_value = table
        .get("filter")
        .ok_or(RuleError::MissingField { field: "filter" })?;
    let selector = selector::parse(filter_value, action.context())?;

    Ok(Rule {
        reason,
        selector,
        action,
    })
}

/// Parses a rule's `action` table into exactly one [`Action`].
fn parse_action(action_table: &toml::Table) -> Result<Action, RuleError> {
    const KNOWN: [&str; 3] = ["wanted", "world_source", "propagate"];

    if let Some(unknown) = action_table
        .keys()
        .find(|key| !KNOWN.contains(&key.as_str()))
    {
        return Err(RuleError::UnknownAction(unknown.clone()));
    }

    let mut actions = Vec::new();
    if let Some(value) = action_table.get("wanted") {
        actions.push(Action::Item(ItemAction::Wanted(value.as_bool().ok_or(
            RuleError::WrongShape {
                key: "wanted",
                expected: "a bool",
            },
        )?)));
    }
    if let Some(value) = action_table.get("world_source") {
        actions.push(Action::Item(ItemAction::WorldSource(
            value.as_bool().ok_or(RuleError::WrongShape {
                key: "world_source",
                expected: "a bool",
            })?,
        )));
    }
    if let Some(value) = action_table.get("propagate") {
        actions.push(Action::Recipe(RecipeAction::Propagate(parse_propagate(
            value,
        )?)));
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
    /// An `action` table had more than one recognized action
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
    /// An error inside one `[[rule]]` entry.
    #[error("rule {position}: {source}")]
    InRule {
        /// The rule's position in the file.
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
    use crate::analysis::edges::{Cause, EdgeState, EdgeStates, NodeAttrs};
    use crate::graph_dsl::parse;
    use crate::model::{ItemIndex, RecipeIndex};

    #[test]
    fn wanted_action_marks_matching_items() {
        let (graph, _wanted) = parse("mix: stone_dust -> gtceu:battery");
        let rules = RuleSet::parse(
            r#"
            [[rule]]
            reason = "batteries are useful"
            filter = { id = "/battery$/" }
            action = { wanted = true }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Sorted item order: gtceu:battery=0, stone_dust=1.
        assert!(attrs.is_wanted(ItemIndex::new(0)));
        assert!(!attrs.is_wanted(ItemIndex::new(1)));
        assert!(matches!(
            attrs.wanted_cause(ItemIndex::new(0)),
            Some(Cause::Rule { file: 0, index: 0 })
        ));
    }

    #[test]
    fn wanted_false_unmarks_a_preset_wanted_item() {
        let (graph, _wanted) = parse("mix: stone_dust -> concrete");
        let rules = RuleSet::parse(
            r#"
            [[rule]]
            reason = "don't want this one after all"
            filter = { id = "concrete" }
            action = { wanted = false }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        attrs.set_wanted(ItemIndex::new(0), Cause::Pseudo);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        assert!(!attrs.is_wanted(ItemIndex::new(0)));
    }

    #[test]
    fn world_source_action_marks_matching_items() {
        let (graph, _wanted) = parse("mix: minecraft:iron_ore -> minecraft:iron_ingot");
        let rules = RuleSet::parse(
            r#"
            [[rule]]
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
        let (graph, _wanted) = parse("macerate: ore -> 2 dust + 1 dust");
        let rules = RuleSet::parse(
            r#"
            [[rule]]
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
        let (graph, _wanted) = parse("mix: stone_dust -> concrete");
        let rules = RuleSet::parse(
            r#"
            [[rule]]
            reason = "preset: every dust is wanted"
            filter = { id = "/dust/" }
            action = { wanted = true }

            [[rule]]
            reason = "local: actually, not this one"
            filter = { id = "stone_dust" }
            action = { wanted = false }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);

        rules.apply(&graph, &mut attrs, &mut edges, 0);

        // Sorted item order: concrete=0, stone_dust=1.
        assert!(!attrs.is_wanted(ItemIndex::new(1)), "the later rule wins");
    }

    #[test]
    fn missing_reason_is_an_error() {
        let result = RuleSet::parse(
            r#"
            [[rule]]
            filter = { id = "x" }
            action = { wanted = true }
            "#,
        );

        let error = result.expect_err("rule 0 is missing `reason`");
        assert!(matches!(
            error,
            RuleError::InRule { position: 0, source }
                if matches!(*source, RuleError::MissingField { field: "reason" })
        ));
    }

    #[test]
    fn mixing_item_and_recipe_actions_is_an_error() {
        let result = RuleSet::parse(
            r#"
            [[rule]]
            reason = "nonsense"
            filter = { id = "x" }
            action = { wanted = true, propagate = "drop" }
            "#,
        );

        let error = result.expect_err("wanted and propagate together is more than one action");
        assert!(matches!(
            error,
            RuleError::InRule { position: 0, source } if matches!(*source, RuleError::MultipleActions)
        ));
    }

    #[test]
    fn a_selector_invalid_for_its_action_context_is_an_error() {
        let result = RuleSet::parse(
            r#"
            [[rule]]
            reason = "tag doesn't apply to a recipe-targeting rule"
            filter = { tag = "forge:tools" }
            action = { propagate = "drop" }
            "#,
        );

        let error = result.expect_err("tag doesn't apply to a Context::Recipe selector");
        assert!(matches!(
            error,
            RuleError::InRule { position: 0, source } if matches!(
                *source,
                RuleError::Selector(SelectorError::WrongContext {
                    key: "tag",
                    context: Context::Recipe,
                })
            )
        ));
    }
}
