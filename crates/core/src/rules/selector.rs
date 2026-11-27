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
//! Applying to only recipes: `has_input`, `has_output`.
//!
//! `has_input` and `has_output`'s nested selector can also use `is_catalyst` alongside the
//! item keys above.
//!
//! ```toml
//! # AND: both must match.
//! filter = { mod = "gtceu", id = "/_battery$/" }
//!
//! # OR: either must match.
//! filter = [ { id = "minecraft:dirt" }, { id = "minecraft:coarse_dirt" } ]
//!
//! # NOT: negates the nested selector.
//! filter = { not = { mod = "minecraft" } }
//!
//! # has_input and has_output take a nested selector, either of which can also
//! # use is_catalyst.
//! filter = { has_output = { tag = "forge:dusts/rare_earth" } }
//! filter = { type = "gtceu:extruder", has_input = { id = "gtceu:wire_extruder_mold", is_catalyst = true } }
//! ```

use fancy_regex::Regex;

use crate::model::{Graph, ItemIndex, ItemKind, RecipeIndex};

/// Which kind of thing a [`Selector`] is being matched against.
///
/// Needed for determining which selector keys are valid (see module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// An item or fluid.
    Item,
    /// A recipe.
    Recipe,
    /// An item sitting in one of a recipe's input or output slots. Like [`Self::Item`],
    /// but also allows `is_catalyst`.
    Slot,
}

impl Context {
    /// A human-readable name for error messages.
    const fn describe(self) -> &'static str {
        match self {
            Self::Item => "an item",
            Self::Recipe => "a recipe",
            Self::Slot => "an item sitting in a recipe's slot",
        }
    }
}

/// Which selector keys [`parse_clause`] accepts in which [`Context`]s.
const KEY_CONTEXTS: &[(&str, &[Context])] = &[
    ("not", &[Context::Item, Context::Recipe, Context::Slot]),
    ("id", &[Context::Item, Context::Recipe, Context::Slot]),
    ("mod", &[Context::Item, Context::Recipe, Context::Slot]),
    ("type", &[Context::Item, Context::Recipe, Context::Slot]),
    ("tag", &[Context::Item, Context::Slot]),
    ("has_input", &[Context::Recipe]),
    ("has_output", &[Context::Recipe]),
    ("is_catalyst", &[Context::Slot]),
];

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
    /// for sake of compatibility with KubeJS. Compatibility with the Java regex engine
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
/// TODO: There probably are many other things we want to do, e.g. a filter for chanced non-0
/// non-1 outputs, a selector for EU usage (less than / more than?), and a selector for "there
/// exists a recipe that is almost identical, except it uses that item instead of this one"
/// (this is useful e.g. to say that GT motors are better crafted in a crafting table than an
/// assembler, or to prefer cutter recipes that use lubricant instead of water or distilled
/// water).
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
    /// Matches one of an item's tags.
    Tag(String),
    /// Matches if any input slot has a member matching the nested [`Context::Slot`]
    /// selector, which can also use `is_catalyst`. Recipe-only.
    Input(Box<Self>),
    /// Matches if the output matches the nested [`Context::Slot`] selector, which can
    /// also use `is_catalyst`. Recipe-only.
    Output(Box<Self>),
    /// Matches a slot's chance: catalyst inputs are chance-0, i.e. not consumed. Only
    /// valid inside `has_input`/`has_output`'s nested selector.
    IsCatalyst(bool),
}

/// Parses a selector from its TOML value, validating every key against `context`.
///
/// # Errors
///
/// If `value` isn't a table, array, or string, a key is unknown, a key's value has the wrong
/// shape, or a key doesn't apply in `context` (e.g. `tag` on a recipe).
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
/// Every key becomes a clause, ANDed together.
fn parse_table(table: &toml::Table, context: Context) -> Result<Selector, SelectorError> {
    let clauses = table
        .iter()
        .map(|(key, value)| parse_clause(key, value, context))
        .collect::<Result<_, _>>()?;
    Ok(Selector::All(clauses))
}

/// Parses one `key = value` entry of a selector table.
#[expect(
    clippy::unreachable,
    reason = "key was already validated against KEY_CONTEXTS above"
)]
fn parse_clause(
    raw_key: &str,
    value: &toml::Value,
    context: Context,
) -> Result<Selector, SelectorError> {
    let (key, allowed_contexts) = KEY_CONTEXTS
        .iter()
        .find(|(candidate, _)| *candidate == raw_key)
        .copied()
        .ok_or_else(|| SelectorError::UnknownKey(raw_key.to_owned()))?;
    if !allowed_contexts.contains(&context) {
        return Err(SelectorError::WrongContext { key, context });
    }

    match key {
        "not" => parse(value, context).map(|inner| Selector::Not(Box::new(inner))),
        "id" => string_value(value, "id")
            .and_then(Pattern::parse)
            .map(Selector::Id),
        "mod" => string_value(value, "mod").map(|m| Selector::Mod(m.to_owned())),
        "type" => string_value(value, "type")
            .and_then(Pattern::parse)
            .map(Selector::Type),
        "tag" => string_value(value, "tag").map(|tag| Selector::Tag(tag.to_owned())),
        "has_input" => {
            parse(value, Context::Slot).map(|inner| Selector::Input(Box::new(inner)))
        }
        "has_output" => {
            parse(value, Context::Slot).map(|inner| Selector::Output(Box::new(inner)))
        }
        "is_catalyst" => bool_value(value, "is_catalyst").map(Selector::IsCatalyst),
        _ => unreachable!("key was just looked up in KEY_CONTEXTS"),
        // TODO: Is there no way to rustily exhaustively check a vector of strings?
        // e.g. by using a hashmap instead of a vector?
    }
}

/// `value` as a string, or a [`SelectorError::WrongShape`] naming `key`.
fn string_value<'v>(
    value: &'v toml::Value,
    key: &'static str,
) -> Result<&'v str, SelectorError> {
    value.as_str().ok_or(SelectorError::WrongShape {
        key,
        expected: "a string",
    })
}

/// `value` as a bool, or a [`SelectorError::WrongShape`] naming `key`.
fn bool_value(value: &toml::Value, key: &'static str) -> Result<bool, SelectorError> {
    value.as_bool().ok_or(SelectorError::WrongShape {
        key,
        expected: "a bool",
    })
}

/// The namespace part of an id, i.e. everything before the first `:`. For an id without one,
/// that's `minecraft`, as in Minecraft.
fn namespace_of(id: &str) -> &str {
    id.split_once(':')
        .map_or("minecraft", |(namespace, _rest)| namespace)
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

/// Whether `item` matches `selector`.
///
/// # Panics
///
/// If `item` is out of bounds for `graph`. [`parse`] never constructs a recipe-only key
/// (`has_input`/`has_output`) or `is_catalyst` for [`Context::Item`], so those arms should be
/// unreachable by construction.
#[inline]
#[must_use]
#[expect(
    clippy::unreachable,
    reason = "parse() enforces selector/context agreement before this ever runs"
)]
pub fn matches_item(selector: &Selector, graph: &Graph, item: ItemIndex) -> bool {
    let subject = graph.item(item);
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
        Selector::Input(_) | Selector::Output(_) | Selector::IsCatalyst(_) => {
            unreachable!("parse() only builds these for Context::Recipe or Context::Slot")
        }
    }
}

/// Whether `recipe` matches `selector`.
///
/// # Panics
///
/// If `recipe` is out of bounds for `graph`, or `recipe`'s type index is out of bounds for
/// `graph.recipe_types()`. [`parse`] never constructs the item-only `tag` key or `is_catalyst`
/// for [`Context::Recipe`], so those arms are unreachable by construction.
#[inline]
#[must_use]
#[expect(
    clippy::unreachable,
    reason = "parse() enforces selector/context agreement before this ever runs"
)]
pub fn matches_recipe(selector: &Selector, graph: &Graph, recipe: RecipeIndex) -> bool {
    let subject = graph.recipe(recipe);
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
        Selector::Tag(_) => {
            unreachable!("parse() only builds Tag for Context::Item or Context::Slot")
        }
        Selector::Input(inner) => subject.inputs().iter().any(|slot| {
            slot.members()
                .iter()
                .any(|&member| matches_slot_member(inner, graph, slot.chance(), member))
        }),
        Selector::Output(inner) => subject
            .outputs()
            .iter()
            .any(|output| matches_slot_member(inner, graph, output.chance(), output.item())),
        Selector::IsCatalyst(_) => {
            unreachable!("parse() only builds IsCatalyst for Context::Slot")
        }
    }
}

/// Whether `member`, sitting in a slot of chance `chance`, matches `selector`. Used for
/// `has_input`/`has_output`'s nested [`Context::Slot`] selector.
///
/// # Panics
///
/// If `member` is out of bounds for `graph`. [`parse`] never constructs the recipe-only keys
/// (`has_input`/`has_output`) for [`Context::Slot`], so those arms are unreachable by
/// construction.
#[inline]
#[must_use]
#[expect(
    clippy::unreachable,
    reason = "parse() enforces selector/context agreement before this ever runs"
)]
fn matches_slot_member(
    selector: &Selector,
    graph: &Graph,
    chance: f32,
    member: ItemIndex,
) -> bool {
    match selector {
        Selector::All(clauses) => clauses
            .iter()
            .all(|clause| matches_slot_member(clause, graph, chance, member)),
        Selector::Any(clauses) => clauses
            .iter()
            .any(|clause| matches_slot_member(clause, graph, chance, member)),
        Selector::Not(inner) => !matches_slot_member(inner, graph, chance, member),
        Selector::IsCatalyst(expected) => is_catalyst(chance) == *expected,
        Selector::Id(_) | Selector::Mod(_) | Selector::Type(_) | Selector::Tag(_) => {
            matches_item(selector, graph, member)
        }
        Selector::Input(_) | Selector::Output(_) => {
            unreachable!("parse() only builds these for Context::Recipe")
        }
    }
}

/// Whether a slot's chance marks it as a catalyst (chance exactly 0).
const fn is_catalyst(chance: f32) -> bool {
    chance == 0.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Context, Selector, SelectorError, matches_item, matches_recipe, parse};
    use crate::graph_dsl::{item, parse as parse_graph, recipe};
    use crate::model::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, Slot};

    fn selector(toml: &str, context: Context) -> Selector {
        let value: toml::Value = toml::from_str(toml).expect("valid test TOML");
        let wrapped = value
            .get("filter")
            .expect("test TOML has a top-level `filter` key")
            .clone();
        parse(&wrapped, context).expect("valid test selector")
    }

    /// Every non-blank, non-comment line inside this module's own ```` ```toml ```` doc block.
    fn module_doc_filter_examples() -> Vec<String> {
        super::super::doc_comment_fenced_blocks(include_str!("selector.rs"), "toml")
            .iter()
            .flat_map(|block| block.lines())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn every_module_doc_filter_example_is_valid() {
        let examples = module_doc_filter_examples();
        assert!(
            !examples.is_empty(),
            "expected to find some ```toml filter examples in the module doc"
        );
        for example in &examples {
            let _parsed = selector(example, Context::Recipe);
        }
    }

    #[test]
    fn exact_id_matches_only_that_id() {
        let (graph, _required) = parse_graph("oak_planks: oak_log -> 4 oak_planks");
        let select = selector(r#"filter = { id = "oak_planks" }"#, Context::Item);

        assert!(matches_item(&select, &graph, item(&graph, "oak_planks")));
        assert!(!matches_item(&select, &graph, item(&graph, "oak_log")));
    }

    #[test]
    fn regexes_are_unanchored_and_match_anywhere_in_the_id() {
        let (graph, _required) =
            parse_graph("coarse_dirt: 2 dirt + 2 gravel -> 4 coarse_dirt");
        let select = selector(r#"filter = { id = "/dirt/" }"#, Context::Item);

        assert!(
            matches_item(&select, &graph, item(&graph, "coarse_dirt")),
            "unanchored: /dirt/ matches anywhere in the id, not just the whole string"
        );
        assert!(matches_item(&select, &graph, item(&graph, "dirt")));
        assert!(!matches_item(&select, &graph, item(&graph, "gravel")));
    }

    #[test]
    fn slash_wrapped_id_is_a_regex() {
        let (graph, _required) =
            parse_graph("iron_ingot_from_smelting_raw_iron: raw_iron -> iron_ingot");
        let select = selector(r#"filter = { id = "/^raw_/" }"#, Context::Item);

        assert!(matches_item(&select, &graph, item(&graph, "raw_iron")));
        assert!(!matches_item(&select, &graph, item(&graph, "iron_ingot")));
    }

    #[test]
    fn mod_matches_the_namespace_only() {
        let (graph, _required) = parse_graph(
            "gtceu:macerator/macerate_cobblestone: cobblestone -> gtceu:stone_dust",
        );
        let select = selector(r#"filter = { mod = "gtceu" }"#, Context::Item);

        assert!(matches_item(
            &select,
            &graph,
            item(&graph, "gtceu:stone_dust")
        ));
        assert!(!matches_item(&select, &graph, item(&graph, "cobblestone")));
    }

    #[test]
    fn object_is_and_array_is_or() {
        let (graph, _required) = parse_graph(
            "
            gtceu:macerator/macerate_cobblestone: cobblestone -> gtceu:stone_dust
            gtceu:macerator/macerate_iron_ingot:  iron_ingot -> gtceu:iron_dust
            ",
        );
        let and_select = selector(
            r#"filter = { mod = "minecraft", id = "/iron/" }"#,
            Context::Item,
        );
        let or_select = selector(
            r#"filter = [ { id = "cobblestone" }, { id = "iron_ingot" } ]"#,
            Context::Item,
        );

        assert!(matches_item(
            &and_select,
            &graph,
            item(&graph, "iron_ingot")
        ));
        assert!(
            !matches_item(&and_select, &graph, item(&graph, "gtceu:iron_dust")),
            "matches the id, but not the mod"
        );
        assert!(
            !matches_item(&and_select, &graph, item(&graph, "cobblestone")),
            "matches the mod, but not the id"
        );
        assert!(matches_item(
            &or_select,
            &graph,
            item(&graph, "cobblestone")
        ));
        assert!(matches_item(&or_select, &graph, item(&graph, "iron_ingot")));
        assert!(!matches_item(
            &or_select,
            &graph,
            item(&graph, "gtceu:iron_dust")
        ));
    }

    #[test]
    fn not_negates_the_inner_selector() {
        let (graph, _required) = parse_graph(
            "
            gtceu:macerator/macerate_cobblestone: cobblestone -> gtceu:stone_dust
            gtceu:macerator/macerate_iron_ingot:  iron_ingot -> gtceu:iron_dust
            ",
        );
        let select = selector(r#"filter = { not = { mod = "minecraft" } }"#, Context::Item);

        assert!(!matches_item(&select, &graph, item(&graph, "cobblestone")));
        assert!(!matches_item(&select, &graph, item(&graph, "iron_ingot")));
        assert!(matches_item(
            &select,
            &graph,
            item(&graph, "gtceu:stone_dust")
        ));
        assert!(matches_item(
            &select,
            &graph,
            item(&graph, "gtceu:iron_dust")
        ));
    }

    /// The real `gtceu:extruder/extrude_iron_to_rod` takes an iron ingot and a mold (catalyst,
    /// chance 0) and makes 2 iron rods.
    fn extruder_graph() -> Graph {
        // Items sorted by id: gtceu:iron_rod=0, gtceu:rod_extruder_mold=1, iron_ingot=2.
        Graph::new(
            1,
            vec![
                Item::new("gtceu:iron_rod".to_owned(), ItemKind::Item, BTreeSet::new()),
                Item::new(
                    "gtceu:rod_extruder_mold".to_owned(),
                    ItemKind::Item,
                    BTreeSet::new(),
                ),
                Item::new("iron_ingot".to_owned(), ItemKind::Item, BTreeSet::from([0])),
            ],
            vec![Recipe::new(
                "gtceu:extruder/extrude_iron_to_rod".to_owned(),
                0,
                vec![
                    Slot::new(BTreeSet::from([ItemIndex::new(2)]), Some(0), 1, 1.0),
                    Slot::new(BTreeSet::from([ItemIndex::new(1)]), None, 1, 0.0),
                ],
                vec![Output::new(ItemIndex::new(0), 2, 1.0)],
                None,
                None,
            )],
            vec!["forge:ingots/iron".to_owned()],
            vec!["gtceu:extruder".to_owned()],
        )
    }

    const EXTRUDE: &str = "gtceu:extruder/extrude_iron_to_rod";

    #[test]
    fn tag_matches_items_carrying_that_tag() {
        let graph = extruder_graph();
        let select = selector(r#"filter = { tag = "forge:ingots/iron" }"#, Context::Item);

        assert!(matches_item(&select, &graph, item(&graph, "iron_ingot")));
        assert!(!matches_item(
            &select,
            &graph,
            item(&graph, "gtceu:rod_extruder_mold")
        ));
    }

    #[test]
    fn tag_on_a_recipe_selector_is_an_error() {
        let value: toml::Value =
            toml::from_str(r#"filter = { tag = "forge:ingots" }"#).expect("valid test TOML");
        let wrapped = value.get("filter").expect("has filter").clone();

        let error = parse(&wrapped, Context::Recipe)
            .expect_err("tag doesn't apply to a recipe selector");

        assert!(matches!(
            error,
            SelectorError::WrongContext {
                key: "tag",
                context: Context::Recipe,
            }
        ));
    }

    #[test]
    fn type_matches_item_kind_for_items() {
        let graph = Graph::new(
            1,
            vec![
                Item::new("dirt".to_owned(), ItemKind::Item, BTreeSet::new()),
                Item::new("water".to_owned(), ItemKind::Fluid, BTreeSet::new()),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let select = selector(r#"filter = { type = "fluid" }"#, Context::Item);

        assert!(matches_item(&select, &graph, item(&graph, "water")));
        assert!(!matches_item(&select, &graph, item(&graph, "dirt")));
    }

    #[test]
    fn type_matches_recipe_type_for_recipes() {
        let graph = extruder_graph();
        let select = selector(r#"filter = { type = "gtceu:extruder" }"#, Context::Recipe);
        let other = selector(r#"filter = { type = "smelting" }"#, Context::Recipe);

        assert!(matches_recipe(&select, &graph, recipe(&graph, EXTRUDE)));
        assert!(!matches_recipe(&other, &graph, recipe(&graph, EXTRUDE)));
    }

    #[test]
    fn has_input_matches_recipes_with_a_matching_slot_member() {
        let (graph, _required) =
            parse_graph("iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe");
        let select = selector(
            r#"filter = { has_input = { id = "stick" } }"#,
            Context::Recipe,
        );
        let output = selector(
            r#"filter = { has_input = { id = "iron_pickaxe" } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(
            &select,
            &graph,
            recipe(&graph, "iron_pickaxe")
        ));
        assert!(
            !matches_recipe(&output, &graph, recipe(&graph, "iron_pickaxe")),
            "the pickaxe is an output, not an input"
        );
    }

    #[test]
    fn has_output_matches_recipes_with_a_matching_output() {
        let (graph, _required) =
            parse_graph("iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe");
        let select = selector(
            r#"filter = { has_output = { id = "iron_pickaxe" } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(
            &select,
            &graph,
            recipe(&graph, "iron_pickaxe")
        ));
    }

    #[test]
    fn is_catalyst_true_matches_only_the_chance_zero_input_member() {
        let graph = extruder_graph();
        let select = selector(
            r#"filter = { has_input = { id = "gtceu:rod_extruder_mold", is_catalyst = true } }"#,
            Context::Recipe,
        );
        let wrong_item = selector(
            r#"filter = { has_input = { id = "iron_ingot", is_catalyst = true } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(&select, &graph, recipe(&graph, EXTRUDE)));
        assert!(
            !matches_recipe(&wrong_item, &graph, recipe(&graph, EXTRUDE)),
            "iron_ingot's slot has chance 1.0, not 0.0"
        );
    }

    #[test]
    fn is_catalyst_false_matches_only_the_chance_one_input_member() {
        let graph = extruder_graph();
        let select = selector(
            r#"filter = { has_input = { id = "iron_ingot", is_catalyst = false } }"#,
            Context::Recipe,
        );
        let wrong_item = selector(
            r#"filter = { has_input = { id = "gtceu:rod_extruder_mold", is_catalyst = false } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(&select, &graph, recipe(&graph, EXTRUDE)));
        assert!(
            !matches_recipe(&wrong_item, &graph, recipe(&graph, EXTRUDE)),
            "the mold's slot has chance 0.0, which is_catalyst = false excludes"
        );
    }

    #[test]
    fn is_catalyst_matches_has_output_s_chance_too() {
        let graph = extruder_graph();
        let select = selector(
            r#"filter = { has_output = { id = "gtceu:iron_rod", is_catalyst = false } }"#,
            Context::Recipe,
        );

        assert!(
            matches_recipe(&select, &graph, recipe(&graph, EXTRUDE)),
            "is_catalyst reads the output's own chance, not just an input slot's"
        );
    }

    #[test]
    fn is_catalyst_outside_has_input_or_has_output_is_an_error() {
        let value: toml::Value =
            toml::from_str("filter = { is_catalyst = true }").expect("valid test TOML");
        let wrapped = value.get("filter").expect("has filter").clone();

        let error = parse(&wrapped, Context::Item).expect_err(
            "is_catalyst only applies inside has_input/has_output's nested selector",
        );

        assert!(matches!(
            error,
            SelectorError::WrongContext {
                key: "is_catalyst",
                context: Context::Item,
            }
        ));
    }

    #[test]
    fn has_input_still_allows_tag_on_its_nested_selector() {
        let graph = extruder_graph();
        let select = selector(
            r#"filter = { has_input = { tag = "forge:ingots/iron" } }"#,
            Context::Recipe,
        );

        assert!(matches_recipe(&select, &graph, recipe(&graph, EXTRUDE)));
    }

    #[test]
    fn unknown_key_is_an_error() {
        let value: toml::Value =
            toml::from_str(r#"filter = { nonsense = "x" }"#).expect("valid test TOML");
        let wrapped = value.get("filter").expect("has filter").clone();

        let error = parse(&wrapped, Context::Item)
            .expect_err("nonsense is not a recognized selector key");

        assert!(matches!(error, SelectorError::UnknownKey(key) if key == "nonsense"));
    }
}
