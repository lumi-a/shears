//! KubeJS-style recipe-filter selectors.
//!
//! - Object = AND
//! - Array = OR
//! - `not` = negation
//!
//! Applying to both items and recipes: `id`, `mod`, `type`.
//!
//! Applying to only items: `tag`.
//!
//! Applying to only recipes: `input`, `output`, `catalyst`.
//!
//! TODO: Better documentation here, with examples.

use fancy_regex::Regex;

use crate::model::{Graph, Item, ItemIndex, ItemKind, Recipe, RecipeIndex};

/// Which kind of thing a [`Selector`] is being matched against.
///
/// Needed for determining which selector keys are valid (see module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// An item or fluid.
    Item,
    /// A recipe.
    Recipe,
}

impl Context {
    /// A human-readable name for error messages.
    const fn describe(self) -> &'static str {
        match self {
            Self::Item => "an item",
            Self::Recipe => "a recipe",
        }
    }
}

/// An exact id, or a `/regex/`-wrapped pattern.
#[derive(Debug)]
pub enum Pattern {
    /// Matches only this exact string.
    Exact(String),
    /// Matches anything this regex matches.
    Regex(Regex),
}

impl Pattern {
    /// Parses `raw` as a pattern:
    /// If it's wrapped in `/.../`, it's treated as a regex, anything else is exact.
    ///
    /// The regexes are unanchored, i.e. `/dirt/` will also match `coarse_dirt`. This is
    /// for sake of compatibility with `KubeJS`. Compatibility with the Java regex engine
    /// is mostly given, but not guaranteed. This may be changed in the future.
    ///
    /// The regex crate used is ([`fancy_regex`]), so lookaround and backreferences are
    /// supported.
    ///
    /// # Errors
    ///
    /// If `raw` is slash-wrapped but its body isn't a valid regex.
    #[inline]
    pub fn parse(raw: &str) -> Result<Self, SelectorError> {
        raw.strip_prefix('/')
            .and_then(|rest| rest.strip_suffix('/'))
            .map_or_else(
                || Ok(Self::Exact(raw.to_owned())),
                |body| {
                    Regex::new(body).map(Self::Regex).map_err(|source| {
                        SelectorError::InvalidRegex {
                            pattern: body.to_owned(),
                            source,
                        }
                    })
                },
            )
    }

    /// Whether `subject` matches this pattern.
    ///
    /// A regex that fails at match time (fancy-regex's lookaround can run out of its
    /// backtracking budget on pathological patterns) counts as not matching, rather than
    /// threading a `Result` through every selector in the tree for a failure mode that
    /// only a deliberately malicious or broken regex would hit.
    ///
    /// TODO: Perhaps at least a warning printed to console in debug mode would be nice.
    fn matches(&self, subject: &str) -> bool {
        match self {
            Self::Exact(exact) => exact == subject,
            Self::Regex(regex) => regex.is_match(subject).unwrap_or(false),
        }
    }
}

/// A problem while parsing a selector out of the TOML.
#[expect(
    clippy::module_name_repetitions,
    reason = "clearest name for this type, unfortunately."
)]
#[derive(Debug, thiserror::Error)]
pub enum SelectorError {
    /// A `/.../`-wrapped pattern isn't a valid regex.
    #[error("invalid regex /{pattern}/: {source}")]
    InvalidRegex {
        /// The regex body without the wrapping slashes.
        pattern: String,
        /// Why the regex failed to compile.
        #[source]
        source: fancy_regex::Error,
    },
    /// A selector value had the wrong TOML shape for its key.
    #[error("`{key}` should be {expected}")]
    WrongShape {
        /// The key whose value was malformed.
        key: &'static str,
        /// What shape was expected instead.
        expected: &'static str,
    },
    /// A key that exists, but doesn't apply in this [`Context`].
    #[error("`{key}` doesn't apply to {}", context.describe())]
    WrongContext {
        /// The key that doesn't apply here.
        key: &'static str,
        /// The context it was used in.
        context: Context,
    },
    /// A selector had a key this crate doesn't recognize.
    #[error("unknown selector key `{0}`")]
    UnknownKey(String),
}

/// A selector tree, already parsed and validated for its [`Context`].
///
/// TODO: There probably are many other things we want to do, e.g. a filter for chanced
/// non-0 non-1 outputs, a selector for EU usage (less than / more than?), and a selector
/// for "there exists a recipe that is almost identical, except it uses that item instead
/// of this one" (this is useful e.g. to say that GT motors are better crafted in a
/// crafting table than an assembler, or to prefer cutter recipes that use lubricant
/// instead of water or distilled water).
#[derive(Debug)]
pub enum Selector {
    /// Every child must match (a TOML object).
    All(Vec<Self>),
    /// Any child must match (a TOML array).
    Any(Vec<Self>),
    /// The child must not match.
    Not(Box<Self>),
    /// Matches the subject's full namespaced id.
    Id(Pattern),
    /// Matches the subject id's namespace, i.e. the part before `:`, for a string.
    ///
    /// This can also be done by matching on Id with e.g. `/^minecraft:/`, but this is
    /// more legible.
    Mod(String),
    /// Matches an item's kind (`"item"`/`"fluid"`) or a recipe's type id.
    Type(Pattern),
    /// Matches one of an item's tags. Item-only.
    Tag(String),
    /// Matches if any input slot has a member matching the nested selector. Recipe-only.
    Input(Box<Self>),
    /// Matches if any output matches the nested selector. Recipe-only.
    Output(Box<Self>),
    /// Matches if any catalyst (a chance-0 input) matches the nested selector.
    /// Recipe-only.
    Catalyst(Box<Self>),
}

/// Parses a selector from its TOML value, validating every key against `context`.
///
/// # Errors
///
/// If `value` isn't a table, array, or string, a key is unknown, a key's value has the
/// wrong shape, or a key doesn't apply in `context` (e.g. `tag` on a recipe).
#[inline]
pub fn parse(value: &toml::Value, context: Context) -> Result<Selector, SelectorError> {
    match value {
        toml::Value::Array(items) => items
            .iter()
            .map(|item| parse(item, context))
            .collect::<Result<_, _>>()
            .map(Selector::Any),
        toml::Value::String(id) => Pattern::parse(id).map(Selector::Id),
        toml::Value::Table(table) => parse_table(table, context),
        toml::Value::Boolean(_)
        | toml::Value::Integer(_)
        | toml::Value::Float(_)
        | toml::Value::Datetime(_) => Err(SelectorError::WrongShape {
            key: "<selector>",
            expected: "a table, an array, or a string",
        }),
    }
}

/// Parses one selector table.
///
/// Every key becomes a clause, `AND`ed together.
fn parse_table(table: &toml::Table, context: Context) -> Result<Selector, SelectorError> {
    let clauses = table
        .iter()
        .map(|(key, value)| parse_clause(key, value, context))
        .collect::<Result<_, _>>()?;
    Ok(Selector::All(clauses))
}

/// Parses one `key = value` entry of a selector table.
fn parse_clause(
    key: &str,
    value: &toml::Value,
    context: Context,
) -> Result<Selector, SelectorError> {
    match key {
        "not" => parse(value, context).map(|inner| Selector::Not(Box::new(inner))),
        "id" => string_value(value, "id")
            .and_then(Pattern::parse)
            .map(Selector::Id),
        "mod" => string_value(value, "mod").map(|m| Selector::Mod(m.to_owned())),
        "type" => string_value(value, "type")
            .and_then(Pattern::parse)
            .map(Selector::Type),
        "tag" => {
            require_context(context, Context::Item, "tag")?;
            string_value(value, "tag").map(|tag| Selector::Tag(tag.to_owned()))
        }
        "input" => {
            require_context(context, Context::Recipe, "input")?;
            parse(value, Context::Item).map(|inner| Selector::Input(Box::new(inner)))
        }
        "output" => {
            require_context(context, Context::Recipe, "output")?;
            parse(value, Context::Item).map(|inner| Selector::Output(Box::new(inner)))
        }
        "catalyst" => {
            require_context(context, Context::Recipe, "catalyst")?;
            parse(value, Context::Item).map(|inner| Selector::Catalyst(Box::new(inner)))
        }
        other => Err(SelectorError::UnknownKey(other.to_owned())),
    }
}

/// `value` as a string, or a [`SelectorError::WrongShape`] naming `key`.
fn string_value<'v>(value: &'v toml::Value, key: &'static str) -> Result<&'v str, SelectorError> {
    value.as_str().ok_or(SelectorError::WrongShape {
        key,
        expected: "a string",
    })
}

/// Errors unless `context` is `expected`.
fn require_context(
    context: Context,
    expected: Context,
    key: &'static str,
) -> Result<(), SelectorError> {
    if context == expected {
        Ok(())
    } else {
        Err(SelectorError::WrongContext { key, context })
    }
}

/// The namespace part of an id, i.e. everything before the first `:`.
fn namespace_of(id: &str) -> &str {
    id.split_once(':')
        .map_or(id, |(namespace, _rest)| namespace)
}

/// The selector-facing name for an [`ItemKind`].
const fn item_kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Item => "item",
        ItemKind::Fluid => "fluid",
    }
}

/// `graph.tags()`'s index for `tag`, if it's interned at all.
fn tag_index(graph: &Graph, tag: &str) -> Option<u32> {
    graph
        .tags()
        .iter()
        .position(|candidate| candidate == tag)
        .map(|position| u32::try_from(position).expect("fewer than u32::MAX tags"))
}

/// The item at `item`.
///
/// TODO: This should just be a function on Graph.
///
/// # Panics
///
/// If `item` is out of bounds for `graph`.
fn item_at(graph: &Graph, item: ItemIndex) -> &Item {
    graph
        .items()
        .get(item.index())
        .expect("item index in bounds for this graph")
}

/// The recipe at `recipe`.
///
/// TODO: This should just be a function on Graph.
///
/// # Panics
///
/// If `recipe` is out of bounds for `graph`.
fn recipe_at(graph: &Graph, recipe: RecipeIndex) -> &Recipe {
    graph
        .recipes()
        .get(recipe.index())
        .expect("recipe index in bounds for this graph")
}

/// Whether `item` matches `selector`.
///
/// # Panics
///
/// If `item` is out of bounds for `graph`. [`parse`] never constructs a recipe-only key
/// (`input`/`output`/`catalyst`) for [`Context::Item`], so those arms should be
/// unreachable by construction.
#[inline]
#[must_use]
#[expect(
    clippy::unreachable,
    reason = "parse() enforces selector/context agreement before this ever runs"
)]
pub fn matches_item(selector: &Selector, graph: &Graph, item: ItemIndex) -> bool {
    let subject = item_at(graph, item);
    match selector {
        Selector::All(clauses) => clauses
            .iter()
            .all(|clause| matches_item(clause, graph, item)),
        Selector::Any(clauses) => clauses
            .iter()
            .any(|clause| matches_item(clause, graph, item)),
        Selector::Not(inner) => !matches_item(inner, graph, item),
        Selector::Id(pattern) => pattern.matches(subject.id()),
        Selector::Mod(expected) => namespace_of(subject.id()) == expected,
        Selector::Type(pattern) => pattern.matches(item_kind_name(subject.kind())),
        Selector::Tag(tag) => {
            tag_index(graph, tag).is_some_and(|index| subject.tags().contains(&index))
        }
        Selector::Input(_) | Selector::Output(_) | Selector::Catalyst(_) => {
            unreachable!("parse() only builds these for Context::Recipe")
        }
    }
}

/// Whether `recipe` matches `selector`.
///
/// # Panics
///
/// If `recipe` is out of bounds for `graph`, or `recipe`'s type index is out of bounds
/// for `graph.recipe_types()`. [`parse`] never constructs the item-only `tag` key for
/// [`Context::Recipe`], so that arm is unreachable by construction.
#[inline]
#[must_use]
#[expect(
    clippy::unreachable,
    reason = "parse() enforces selector/context agreement before this ever runs"
)]
pub fn matches_recipe(selector: &Selector, graph: &Graph, recipe: RecipeIndex) -> bool {
    let subject = recipe_at(graph, recipe);
    match selector {
        Selector::All(clauses) => clauses
            .iter()
            .all(|clause| matches_recipe(clause, graph, recipe)),
        Selector::Any(clauses) => clauses
            .iter()
            .any(|clause| matches_recipe(clause, graph, recipe)),
        Selector::Not(inner) => !matches_recipe(inner, graph, recipe),
        Selector::Id(pattern) => pattern.matches(subject.id()),
        Selector::Mod(expected) => namespace_of(subject.id()) == expected,
        Selector::Type(pattern) => {
            let rtype = usize::try_from(subject.rtype()).expect("u32 always fits in usize");
            let name = graph
                .recipe_types()
                .get(rtype)
                .expect("recipe type index in bounds for this graph");
            pattern.matches(name)
        }
        Selector::Tag(_) => unreachable!("parse() only builds Tag for Context::Item"),
        Selector::Input(inner) => subject.inputs().iter().any(|slot| {
            slot.members()
                .iter()
                .any(|&member| matches_item(inner, graph, member))
        }),
        Selector::Output(inner) => subject
            .outputs()
            .iter()
            .any(|output| matches_item(inner, graph, output.item())),
        Selector::Catalyst(inner) => subject
            .inputs()
            .iter()
            .filter(|slot| is_catalyst(slot.chance()))
            .any(|slot| {
                slot.members()
                    .iter()
                    .any(|&member| matches_item(inner, graph, member))
            }),
    }
}

/// Whether a slot's chance marks it as a catalyst (chance exactly 0).
const fn is_catalyst(chance: f32) -> bool {
    chance == 0.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{matches_item, matches_recipe, parse, Context, Selector};
    use crate::graph_dsl::parse as parse_graph;
    use crate::model::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, RecipeIndex, Slot};

    fn selector(toml: &str, context: Context) -> Selector {
        let value: toml::Value = toml::from_str(toml).expect("valid test TOML");
        let wrapped = value
            .get("match")
            .expect("test TOML has a top-level `match` key")
            .clone();
        parse(&wrapped, context).expect("valid test selector")
    }

    #[test]
    fn exact_id_matches_only_that_id() {
        let (graph, _roots) = parse_graph("mix: stone_dust -> concrete");
        let select = selector(r#"match = { id = "concrete" }"#, Context::Item);

        assert!(matches_item(&select, &graph, ItemIndex::new(0)));
        assert!(!matches_item(&select, &graph, ItemIndex::new(1)));
    }

    #[test]
    fn regexes_are_unanchored_and_match_anywhere_in_the_id() {
        let (graph, _roots) = parse_graph("mix: coarse_dirt + dirt -> concrete");
        let select = selector(r#"match = { id = "/dirt/" }"#, Context::Item);

        // Sorted item order: coarse_dirt=0, concrete=1.
        assert!(
            matches_item(&select, &graph, ItemIndex::new(0)),
            "unanchored: /dirt/ matches anywhere in the id, not just the whole string"
        );
        assert!(!matches_item(&select, &graph, ItemIndex::new(1))); // concrete
        assert!(matches_item(&select, &graph, ItemIndex::new(2))); // dirt
    }

    #[test]
    fn slash_wrapped_id_is_a_regex() {
        let (graph, _roots) = parse_graph("mix: gtceu:dust_tiny -> gtceu:dust_small");
        let select = selector(r#"match = { id = "/^gtceu:dust_/" }"#, Context::Item);

        // Sorted item order: gtceu:dust_small=0, gtceu:dust_tiny=1.
        assert!(matches_item(&select, &graph, ItemIndex::new(0)));
        assert!(matches_item(&select, &graph, ItemIndex::new(1)));
    }

    #[test]
    fn mod_matches_the_namespace_only() {
        let (graph, _roots) = parse_graph("mix: minecraft:dirt -> gtceu:dust");
        let select = selector(r#"match = { mod = "gtceu" }"#, Context::Item);

        // Sorted item order: gtceu:dust=0, minecraft:dirt=1.
        assert!(matches_item(&select, &graph, ItemIndex::new(0)));
        assert!(!matches_item(&select, &graph, ItemIndex::new(1)));
    }

    #[test]
    fn object_is_and_array_is_or() {
        let (graph, _roots) = parse_graph("mix: minecraft:dirt + minecraft:sand -> gtceu:dust");
        let and_select = selector(
            r#"match = { mod = "minecraft", id = "/dirt/" }"#,
            Context::Item,
        );
        let or_select = selector(
            r#"match = [ { id = "minecraft:dirt" }, { id = "minecraft:sand" } ]"#,
            Context::Item,
        );

        // Sorted item order: gtceu:dust=0, minecraft:dirt=1, minecraft:sand=2.
        assert!(matches_item(&and_select, &graph, ItemIndex::new(1)));
        assert!(!matches_item(&and_select, &graph, ItemIndex::new(2)));
        assert!(matches_item(&or_select, &graph, ItemIndex::new(1)));
        assert!(matches_item(&or_select, &graph, ItemIndex::new(2)));
    }

    #[test]
    fn not_negates_the_inner_selector() {
        let (graph, _roots) = parse_graph("mix: minecraft:dirt -> minecraft:sand");
        let select = selector(r#"match = { not = { mod = "minecraft" } }"#, Context::Item);

        assert!(!matches_item(&select, &graph, ItemIndex::new(0)));
        assert!(!matches_item(&select, &graph, ItemIndex::new(1)));
    }

    #[test]
    fn tag_matches_items_carrying_that_tag() {
        let graph = Graph::new(
            1,
            vec![
                Item::new(
                    "gtceu:iron_ingot".to_owned(),
                    ItemKind::Item,
                    BTreeSet::from([0]),
                ),
                Item::new("gtceu:dirt".to_owned(), ItemKind::Item, BTreeSet::new()),
            ],
            Vec::new(),
            vec!["forge:ingots".to_owned()],
            Vec::new(),
        );
        let select = selector(r#"match = { tag = "forge:ingots" }"#, Context::Item);

        assert!(matches_item(&select, &graph, ItemIndex::new(0)));
        assert!(!matches_item(&select, &graph, ItemIndex::new(1)));
    }

    #[test]
    fn tag_on_a_recipe_selector_is_an_error() {
        let value: toml::Value =
            toml::from_str(r#"match = { tag = "forge:ingots" }"#).expect("valid test TOML");
        let wrapped = value.get("match").expect("has match").clone();

        let error = parse(&wrapped, Context::Recipe);

        assert!(error.is_err(), "tag doesn't apply to a recipe selector");
    }

    #[test]
    fn type_matches_item_kind_for_items() {
        let graph = Graph::new(
            1,
            vec![
                Item::new(
                    "minecraft:water".to_owned(),
                    ItemKind::Fluid,
                    BTreeSet::new(),
                ),
                Item::new("minecraft:dirt".to_owned(), ItemKind::Item, BTreeSet::new()),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let select = selector(r#"match = { type = "fluid" }"#, Context::Item);

        assert!(matches_item(&select, &graph, ItemIndex::new(0)));
        assert!(!matches_item(&select, &graph, ItemIndex::new(1)));
    }

    #[test]
    fn type_matches_recipe_type_for_recipes() {
        let graph = Graph::new(
            1,
            vec![Item::new(
                "minecraft:iron_ingot".to_owned(),
                ItemKind::Item,
                BTreeSet::new(),
            )],
            vec![Recipe::new(
                "r".to_owned(),
                0,
                Vec::new(),
                vec![Output::new(ItemIndex::new(0), 1, 1.0)],
                None,
                None,
            )],
            Vec::new(),
            vec!["gtceu:extruder".to_owned()],
        );
        let select = selector(r#"match = { type = "gtceu:extruder" }"#, Context::Recipe);

        assert!(matches_recipe(&select, &graph, RecipeIndex::new(0)));
    }

    #[test]
    fn input_matches_recipes_with_a_matching_input_member() {
        let (graph, _roots) = parse_graph("mix: stone_dust + water -> concrete");
        let select = selector(
            r#"match = { input = { id = "stone_dust" } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(&select, &graph, RecipeIndex::new(0)));
    }

    #[test]
    fn output_matches_recipes_with_a_matching_output() {
        let (graph, _roots) = parse_graph("mix: stone_dust + water -> concrete");
        let select = selector(
            r#"match = { output = { id = "concrete" } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(&select, &graph, RecipeIndex::new(0)));
    }

    #[test]
    fn catalyst_matches_only_chance_zero_input_members() {
        let graph = Graph::new(
            1,
            vec![
                Item::new("gtceu:mold".to_owned(), ItemKind::Item, BTreeSet::new()),
                Item::new("gtceu:wire".to_owned(), ItemKind::Item, BTreeSet::new()),
                Item::new("gtceu:ingot".to_owned(), ItemKind::Item, BTreeSet::new()),
            ],
            vec![Recipe::new(
                "extrude".to_owned(),
                0,
                vec![
                    Slot::new(BTreeSet::from([ItemIndex::new(0)]), None, 1, 0.0),
                    Slot::new(BTreeSet::from([ItemIndex::new(2)]), None, 1, 1.0),
                ],
                vec![Output::new(ItemIndex::new(1), 1, 1.0)],
                None,
                None,
            )],
            Vec::new(),
            vec!["gtceu:extruder".to_owned()],
        );
        let select = selector(
            r#"match = { catalyst = { id = "gtceu:mold" } }"#,
            Context::Recipe,
        );
        let not_a_catalyst = selector(
            r#"match = { catalyst = { id = "gtceu:ingot" } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(&select, &graph, RecipeIndex::new(0)));
        assert!(!matches_recipe(
            &not_a_catalyst,
            &graph,
            RecipeIndex::new(0)
        ));
    }

    #[test]
    fn unknown_key_is_an_error() {
        let value: toml::Value =
            toml::from_str(r#"match = { nonsense = "x" }"#).expect("valid test TOML");
        let wrapped = value.get("match").expect("has match").clone();

        let _: super::SelectorError =
            parse(&wrapped, Context::Item).expect_err("nonsense is not a recognized selector key");
    }
}
