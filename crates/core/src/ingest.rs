//! Turns a `/kubejs export` into a `model::Graph`.
//!
//! Problems turn into `Diagnostic`s in the returned `ParseReport`.
//!
//! No item or fluid id's index is final until every recipe file has been seen. Everything
//! passes through `ItemInterner` during a single pass over the recipe files. The registries
//! (`registries/item.json`, `registries/fluid.json`) and the tag files are still read first,
//! but only to get the plain sets of known ids as strings. Once every recipe has been seen,
//! `ItemInterner::finish` merges every referenced id with every registered one, sorts
//! the result, and hands back a table to remap every provisional index to its final one.
//!
//! This also means no id's reference can ever fail to resolve during parsing. A typo'd item id
//! gets interned exactly like a real one and is reported afterwards once the full registry
//! is available to check against.

pub mod find_root_directory;
mod ingredient;
mod interner;
mod path;
mod recipes;
pub mod shapes;
mod tags;
mod vanilla;

use core::cell::RefCell;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use self::ingredient::Leaf;
use self::interner::{ItemInterner, remap_index, remap_item_indices, resolve_referenced};
use self::recipes::parse_recipes;
use self::shapes::ShapesError;
use self::tags::{read_registries_and_tags, resolve_all_tags};
use crate::model::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, Slot};

/// This crate's reference recipe-shape config, as plain TOML text.
///
/// Only really useful for tests, not included by default.
pub const DEFAULT_SHAPES_TOML: &str = include_str!("ingest/shapes.toml");

/// One file from the `/kubejs export`.
#[derive(Debug)]
pub struct RawFile {
    /// Path relative to the export root, separated by `/`, e.g.
    /// `recipes/minecraft/acacia_planks.json`.
    pub path: String,
    /// The file's raw bytes.
    pub bytes: Vec<u8>,
}

/// What a [`Diagnostic`] is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum About {
    /// A file of the export, e.g. a recipe that didn't parse.
    File {
        /// Its path. See in [`RawFile::path`].
        path: String,
    },
    /// A tag.
    Tag {
        /// Its id.
        id: String,
    },
    /// An item or fluid.
    Item {
        /// Its id.
        id: String,
    },
}

impl About {
    /// The file at `path`.
    fn file(path: impl Into<String>) -> Self {
        Self::File { path: path.into() }
    }

    /// The tag `id`.
    fn tag(id: impl Into<String>) -> Self {
        Self::Tag { id: id.into() }
    }

    /// The item or fluid `id`.
    fn item(id: impl Into<String>) -> Self {
        Self::Item { id: id.into() }
    }
}

/// A problem noticed while ingesting.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct Diagnostic {
    /// What this concerns.
    pub about: About,
    /// What went wrong.
    pub message: String,
}

/// Summary of what ingest did.
#[derive(Debug, Default, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ParseReport {
    /// One diagnostic per problem.
    pub diagnostics: Vec<Diagnostic>,
    /// Number of recipe files seen, by their declared `type`.
    pub recipe_type_counts: BTreeMap<String, u32>,
    /// Every registered id that no recipe or tag ever references, sorted.
    pub registered_but_unreferenced: Vec<String>,
}

impl ParseReport {
    /// Records one problem.
    fn fail(&mut self, about: About, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            about,
            message: message.into(),
        });
    }
}

/// A recipe, as read from its JSON, already resolved to provisional ids.
///
/// The type is still a bare string, since the full set of types isn't known until every recipe
/// file has been seen.
#[derive(Debug)]
struct RecipeDraft {
    /// The recipe id, taken from its file path.
    id: String,
    /// Input slots, already tag-expanded (if it's a tag).
    inputs: Vec<Slot>,
    /// Outputs, already interned and resolved.
    outputs: Vec<Output>,
    /// EU/t (energy units per tick) for recipes that have any.
    eu_t: Option<i64>,
    /// Duration in ticks, for recipes that have any.
    ticks: Option<u32>,
}

/// The lookups used while parsing the recipe files.
struct Resolver<'a> {
    /// Index into [`Graph::tags`] for the tags.
    tag_index: &'a BTreeMap<String, u32>,
    /// Resolved, transitively-expanded members for every tag in `tag_index`, already interned
    /// (provisionally, same as every other [`ItemIndex`] here, not final until
    /// [`ItemInterner::finish`]).
    tag_members: &'a BTreeMap<String, &'a BTreeSet<ItemIndex>>,
    /// Every item/fluid id seen so far, interned as it's first referenced. See
    /// [`ItemInterner`].
    interner: &'a RefCell<ItemInterner>,
    /// Ids directly named by some recipe's input leaf or output, as opposed to tag's members.
    /// See `used_tags` for that instead.
    direct_refs: &'a RefCell<BTreeSet<ItemIndex>>,
    /// Index into [`Graph::tags`], for each tag used by some recipe's input leaf.
    used_tags: &'a RefCell<BTreeSet<u32>>,
}

/// Interns every item/fluid id referenced while parsing a recipe (directly, or via a tag's
/// expanded members).
///
/// `id` with Minecraft's default namespace `minecraft:` in front, if it has no namespace.
///
/// Minecraft reads an id without a `:` as a `minecraft:` id, and some recipes rely on that,
/// e.g. `chipped_express`'s stonecutting recipes with `"result": "dirt"`. Only the part before
/// a synthetic id's `{` counts, since an NBT suffix like `{configuration:1}` has `:`s of its
/// own. Ids that already have a namespace are returned as they are, so this can be applied
/// twice.
fn with_namespace(id: &str) -> Cow<'_, str> {
    let base = id.split('{').next().unwrap_or(id);
    if base.contains(':') {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("minecraft:{id}"))
    }
}

/// Turns a raw export into a normalized graph and a parse report.
///
/// Vanilla recipe types only, for now; everything else becomes an "unhandled recipe type"
/// diagnostic rather than being silently dropped (unless `shapes_toml` describes a shape for
/// it instead).
///
/// # Errors
///
/// If `shapes_toml` itself fails to parse.
///
/// # Panics
///
/// Only if a single export has more than [`u32::MAX`] items, tags or recipe types, which isn't
/// a real export this tool will ever see.
#[expect(
    clippy::missing_inline_in_public_items,
    reason = "top-level large function, usually has little reason to be inlined."
)]
pub fn ingest(
    files: &[RawFile],
    shapes_toml: &str,
) -> Result<(Graph, ParseReport), ShapesError> {
    let shapes = shapes::Shapes::parse(shapes_toml)?;
    let mut report = ParseReport::default();

    let (item_keys, fluid_keys, raw_tags) = read_registries_and_tags(files, &mut report);
    let registered: BTreeSet<&str> = item_keys
        .iter()
        .chain(fluid_keys.iter())
        .map(String::as_str)
        .collect();

    // The interner is created before tags are resolved.
    // A tag's members are interned once here, instead of keeping it as tags and letting
    // recipes handle it. Tags are shared across recipes often, so this is more efficient.
    let interner_cell = RefCell::new(ItemInterner::default());
    let (tags, provisional_tag_members) =
        resolve_all_tags(&raw_tags, &registered, &interner_cell, &mut report);
    let tag_index: BTreeMap<String, u32> = tags
        .iter()
        .enumerate()
        .map(|(index, id)| {
            (
                id.clone(),
                u32::try_from(index).expect(
                    "fewer than u32::MAX tags. Please open an issue if this is a problem for you.",
                ),
            )
        })
        .collect();
    let tag_members: BTreeMap<String, &BTreeSet<ItemIndex>> = tags
        .iter()
        .cloned()
        .zip(provisional_tag_members.iter())
        .collect();

    let direct_refs_cell: RefCell<BTreeSet<ItemIndex>> = RefCell::new(BTreeSet::new());
    let used_tags_cell: RefCell<BTreeSet<u32>> = RefCell::new(BTreeSet::new());
    let resolver = Resolver {
        tag_index: &tag_index,
        tag_members: &tag_members,
        interner: &interner_cell,
        direct_refs: &direct_refs_cell,
        used_tags: &used_tags_cell,
    };
    let mut recipe_drafts = parse_recipes(files, &resolver, &shapes, &mut report);
    recipe_drafts.sort_unstable_by(|(_, a), (_, b)| a.id.cmp(&b.id));

    let interner = interner_cell.into_inner();
    let referenced = resolve_referenced(
        &interner,
        direct_refs_cell.into_inner(),
        used_tags_cell.into_inner(),
        &provisional_tag_members,
    );

    let (sorted_keys, remap) =
        interner.finish(item_keys.iter().chain(fluid_keys.iter()).cloned());
    remap_item_indices(&mut recipe_drafts, &remap);

    let tag_members_by_id: Vec<BTreeSet<ItemIndex>> = provisional_tag_members
        .iter()
        .map(|members| {
            members
                .iter()
                .copied()
                .map(|id| remap_index(id, &remap))
                .collect()
        })
        .collect();

    let items = build_items(
        &sorted_keys,
        &item_keys,
        &fluid_keys,
        &referenced,
        &tag_members_by_id,
        &mut report,
    );

    let mut rtypes: Vec<String> = report.recipe_type_counts.keys().cloned().collect();
    rtypes.sort_unstable();
    let rtype_index: BTreeMap<String, u32> = rtypes
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                name.clone(),
                u32::try_from(index).expect("fewer than u32::MAX recipe types. Please open an issue if this is a problem for you."),
            )
        })
        .collect();

    let recipes = recipe_drafts
        .into_iter()
        .map(|(rtype, draft)| {
            let rtype_id = rtype_index
                .get(rtype.as_str())
                .copied()
                .expect("rtype was interned above");
            Recipe::new(
                draft.id,
                rtype_id,
                draft.inputs,
                draft.outputs,
                draft.eu_t,
                draft.ticks,
            )
        })
        .collect();

    Ok((Graph::new(1, items, recipes, tags, rtypes), report))
}

/// Maps a raw file path under `recipes/` to the recipe id it defines.
///
/// For instance, maps `recipes/minecraft/acacia_planks.json` to `minecraft:acacia_planks`.
fn recipe_id_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("recipes/")?.strip_suffix(".json")?;
    let (namespace, name) = rest.split_once('/')?;
    Some(format!("{namespace}:{name}"))
}

/// The path of the file that defines the recipe `id`.
///
/// `None` for an id without a namespace, which no recipe should have?
///
/// # Examples
///
/// ```
/// use shears_core::ingest::recipe_path;
///
/// assert_eq!(
///     recipe_path("minecraft:acacia_planks").as_deref(),
///     Some("recipes/minecraft/acacia_planks.json")
/// );
/// assert_eq!(
///     recipe_path("gtceu:extruder/extrude_iron_to_rod").as_deref(),
///     Some("recipes/gtceu/extruder/extrude_iron_to_rod.json")
/// );
/// assert_eq!(recipe_path("acacia_planks"), None);
/// ```
#[inline]
#[must_use]
pub fn recipe_path(id: &str) -> Option<String> {
    let (namespace, name) = id.split_once(':')?;
    Some(format!("recipes/{namespace}/{name}.json"))
}

/// Builds the final `Item` list, in `sorted_keys`' order.
///
/// Classifies each key by checking `item_keys`/`fluid_keys`.
/// - Both means a registered item/fluid collision. This is reported, and the item wins.
/// - Neither means the key was only ever *referenced*, never registered. Either a synthetic
///   id, or an unknown-to-the-registers item, in which case it's reported.
/// - Also collects every registered id that's in neither `referenced` into
///   [`ParseReport::registered_but_unreferenced`]. These are items or fluids that no recipe
///   ever mentions, either directly or via a tag that some recipe actually uses. If a tag is
///   used in some recipe, then all members of that tag count as referenced.
fn build_items(
    sorted_keys: &[String],
    item_keys: &BTreeSet<String>,
    fluid_keys: &BTreeSet<String>,
    referenced: &BTreeSet<String>,
    tag_members: &[BTreeSet<ItemIndex>],
    report: &mut ParseReport,
) -> Vec<Item> {
    let all_registered: BTreeSet<&String> =
        item_keys.iter().chain(fluid_keys.iter()).collect();
    report.registered_but_unreferenced = all_registered
        .into_iter()
        .filter(|key| !referenced.contains(key.as_str()))
        .cloned()
        .collect();

    // This used to be really costly, we used to scan over every tag (O(item*tags)) for every
    // item, which profiling showed was the largest time sink of ingest. Instead, we iterate
    // over all tags once and track their members in a set.
    let mut tags_by_item: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); sorted_keys.len()];
    for (tag_index, members) in tag_members.iter().enumerate() {
        let tag_index_u32 = u32::try_from(tag_index).expect(
            "fewer than u32::MAX tags. Please open an issue if this is a problem for you.",
        );
        for &member in members {
            let tags = tags_by_item.get_mut(member.index()).expect(
                "every tag member index is within sorted_keys' bounds, having already been \
                 remapped to its final index",
            );
            let _: bool = tags.insert(tag_index_u32);
        }
    }

    sorted_keys
        .iter()
        .zip(tags_by_item)
        .map(|(key, item_tags)| {
            let is_item = item_keys.contains(key);
            let is_fluid = fluid_keys.contains(key);
            let kind = if is_item && is_fluid {
                report.fail(
                    About::item(key),
                    "registered as both an item and a fluid, only the item will be kept",
                );
                ItemKind::Item
            } else if is_fluid {
                ItemKind::Fluid
            } else if is_item {
                ItemKind::Item
            } else {
                if !key.contains('{') {
                    report.fail(
                        About::item(key),
                        "referenced by a recipe, but not a registered item or fluid",
                    );
                }
                ItemKind::Item
            };
            Item::new(key.clone(), kind, item_tags)
        })
        .collect()
}

/// Resolves a flat list of alternatives to the set of [`Item`]s.
fn resolve_leaves(
    leaves: &[Leaf],
    resolver: &Resolver<'_>,
) -> Result<BTreeSet<ItemIndex>, String> {
    let mut out = BTreeSet::new();
    for leaf in leaves {
        match leaf {
            Leaf::Item(key) => {
                let index = resolver.interner.borrow_mut().intern(key);
                let _: bool = resolver.direct_refs.borrow_mut().insert(index);
                let _: bool = out.insert(index);
            }
            Leaf::Tag(tag_id) => {
                let members = resolver
                    .tag_members
                    .get(tag_id.as_str())
                    .ok_or_else(|| format!("unresolved tag {tag_id:?}"))?;
                let tag_position = resolver.tag_index.get(tag_id.as_str()).copied().expect(
                    "tag_index and tag_members are built from the same tags list, so share \
                     the same keys",
                );
                let _: bool = resolver.used_tags.borrow_mut().insert(tag_position);
                out.extend(members.iter().copied());
            }
        }
    }
    Ok(out)
}

/// The tag this ingredient was expanded from.
fn slot_tag(leaves: &[Leaf], resolver: &Resolver<'_>) -> Option<u32> {
    match leaves {
        [Leaf::Tag(tag_id)] => resolver.tag_index.get(tag_id.as_str()).copied(),
        _ => None,
    }
}

/// Resolves one already-parsed ingredient into a fully-consumed input [`Slot`].
fn consumed_slot(
    leaves: &[Leaf],
    count: u32,
    resolver: &Resolver<'_>,
) -> Result<Slot, String> {
    let members = resolve_leaves(leaves, resolver)?;
    let tag = slot_tag(leaves, resolver);
    Ok(Slot::new(members, tag, count, 1.0))
}

/// How far outside `[0, 1]` a chance value may lie.
pub const CHANCE_TOLERANCE: f32 = 1e-5;

/// Validates a chance value read from the export, snapping it to 0 or 1 if it's within
/// [`CHANCE_TOLERANCE`] of either, on both sides.
///
/// The snapping inside the range keeps checks like "this is a catalyst" and "this is
/// definitely consumed" exact, even if some recipe's numbers are slightly off. GTCEu's
/// finest step is 1/10000, ten times the tolerance, so no real chance gets snapped.
///
/// # Errors
///
/// If `raw` is more than [`CHANCE_TOLERANCE`] outside `[0, 1]`. That is, if `raw <
/// -CHANCE_TOLERANCE` or `raw > 1+CHANCE_TOLERANCE`.
///
/// # Examples
///
/// ```
/// use shears_core::ingest::validate_chance;
///
/// // Values inside the range pass through unchanged.
/// assert_eq!(validate_chance(0.0), Ok(0.0));
/// assert_eq!(validate_chance(0.5), Ok(0.5));
/// assert_eq!(validate_chance(1.0), Ok(1.0));
///
/// // Noise within the tolerance snaps to the exact boundary, from either side.
/// assert_eq!(validate_chance(-0.000_001), Ok(0.0));
/// assert_eq!(validate_chance(1.000_001), Ok(1.0));
/// assert_eq!(validate_chance(0.000_001), Ok(0.0));
/// assert_eq!(validate_chance(0.999_999), Ok(1.0));
///
/// // Further out is an error.
/// assert!(validate_chance(-0.1).is_err());
/// assert!(validate_chance(1.1).is_err());
/// ```
#[inline]
pub fn validate_chance(raw: f32) -> Result<f32, String> {
    if raw < -CHANCE_TOLERANCE || raw > 1.0 + CHANCE_TOLERANCE {
        Err(format!("chance {raw} is out of range [0, 1]"))
    } else if raw <= CHANCE_TOLERANCE {
        Ok(0.0)
    } else if raw >= 1.0 - CHANCE_TOLERANCE {
        Ok(1.0)
    } else {
        Ok(raw)
    }
}

/// Resolves a bare output item or fluid id to its provisional [`ItemIndex`]. Never fails,
/// see [`ItemInterner`].
fn resolve_output_id(id: &str, resolver: &Resolver<'_>) -> ItemIndex {
    let index = resolver.interner.borrow_mut().intern(id);
    let _: bool = resolver.direct_refs.borrow_mut().insert(index);
    index
}

/// Owned backing data for a tag-less [`Resolver`].
#[cfg(test)]
#[derive(Default)]
struct PlainResolverParts {
    /// Backing for [`Resolver::interner`].
    interner: RefCell<ItemInterner>,
    /// Backing for [`Resolver::tag_index`]. Always empty.
    tag_index: BTreeMap<String, u32>,
    /// Backing for [`Resolver::tag_members`]. Always empty.
    tag_members: BTreeMap<String, &'static BTreeSet<ItemIndex>>,
    /// Backing for [`Resolver::direct_refs`].
    direct_refs: RefCell<BTreeSet<ItemIndex>>,
    /// Backing for [`Resolver::used_tags`].
    used_tags: RefCell<BTreeSet<u32>>,
}

#[cfg(test)]
impl PlainResolverParts {
    /// Borrows a [`Resolver`] from this.
    fn resolver(&self) -> Resolver<'_> {
        Resolver {
            tag_index: &self.tag_index,
            tag_members: &self.tag_members,
            interner: &self.interner,
            direct_refs: &self.direct_refs,
            used_tags: &self.used_tags,
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "these item ids deliberately contain `{...}` NBT/circuit-configuration \
              id_suffix suffixes, these are not format arguments"
)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        About, DEFAULT_SHAPES_TOML, ParseReport, PlainResolverParts, RawFile, build_items,
        ingest, recipe_id_from_path, recipe_path, resolve_output_id, with_namespace,
    };
    use crate::model::{ItemIndex, ItemKind};

    #[test]
    fn recipe_path_is_the_reverse_of_recipe_id_from_path() {
        let path = "recipes/gtceu/extruder/extrude_iron_to_rod.json";
        let id = recipe_id_from_path(path).expect("a recipe path");

        assert_eq!(id, "gtceu:extruder/extrude_iron_to_rod");
        assert_eq!(recipe_path(&id).as_deref(), Some(path));
    }

    #[test]
    fn an_id_registered_as_both_an_item_and_a_fluid_is_reported() {
        let sorted_keys = vec!["nuclearcraft:plutonium_239".to_owned()];
        let item_keys = BTreeSet::from(["nuclearcraft:plutonium_239".to_owned()]);
        let fluid_keys = BTreeSet::from(["nuclearcraft:plutonium_239".to_owned()]);
        let referenced: BTreeSet<String> = BTreeSet::new();
        let mut report = ParseReport::default();

        let items = build_items(
            &sorted_keys,
            &item_keys,
            &fluid_keys,
            &referenced,
            &[],
            &mut report,
        );

        assert_eq!(items.len(), 1, "the id collapses to one entry");
        assert_eq!(items.first().expect("one item").kind(), ItemKind::Item);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("both an item and a fluid"))
        );
    }

    #[test]
    fn distinct_item_and_fluid_ids_are_reported_as_two_entries_without_a_collision_warning() {
        let sorted_keys = vec![
            "minecraft:iron_ingot".to_owned(),
            "minecraft:water".to_owned(),
        ];
        let item_keys = BTreeSet::from(["minecraft:iron_ingot".to_owned()]);
        let fluid_keys = BTreeSet::from(["minecraft:water".to_owned()]);
        let referenced: BTreeSet<String> = sorted_keys.iter().cloned().collect();
        let mut report = ParseReport::default();

        let items = build_items(
            &sorted_keys,
            &item_keys,
            &fluid_keys,
            &referenced,
            &[],
            &mut report,
        );

        assert_eq!(items.len(), 2);
        assert!(report.diagnostics.is_empty());
        assert_eq!(items.first().expect("two items").kind(), ItemKind::Item);
        assert_eq!(items.get(1).expect("two items").kind(), ItemKind::Fluid);
    }

    #[test]
    fn a_registered_but_unreferenced_id_is_collected_separately_from_diagnostics() {
        let sorted_keys = vec!["minecraft:dirt".to_owned()];
        let item_keys = BTreeSet::from(["minecraft:dirt".to_owned()]);
        let fluid_keys: BTreeSet<String> = BTreeSet::new();
        let referenced: BTreeSet<String> = BTreeSet::new();
        let mut report = ParseReport::default();

        let items = build_items(
            &sorted_keys,
            &item_keys,
            &fluid_keys,
            &referenced,
            &[],
            &mut report,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(
            report.registered_but_unreferenced,
            vec!["minecraft:dirt".to_owned()]
        );
        assert!(
            report.diagnostics.is_empty(),
            "unreferenced isn't a Diagnostic-worthy problem, just a fact: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_referenced_but_unregistered_non_synthetic_id_is_reported_as_unknown() {
        let sorted_keys = vec!["minecraft:made_up_item".to_owned()];
        let item_keys: BTreeSet<String> = BTreeSet::new();
        let fluid_keys: BTreeSet<String> = BTreeSet::new();
        let referenced: BTreeSet<String> = sorted_keys.iter().cloned().collect();
        let mut report = ParseReport::default();

        let items = build_items(
            &sorted_keys,
            &item_keys,
            &fluid_keys,
            &referenced,
            &[],
            &mut report,
        );

        assert_eq!(items.len(), 1, "it still gets an entry, just a flagged one");
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.about
                == About::Item {
                    id: "minecraft:made_up_item".to_owned(),
                }
                && diagnostic
                    .message
                    .contains("not a registered item or fluid")
        }));
    }

    #[test]
    fn a_referenced_synthetic_id_is_not_reported_as_unknown() {
        let sorted_keys = vec!["gtceu:programmed_circuit{configuration:1}".to_owned()];
        let item_keys: BTreeSet<String> = BTreeSet::new();
        let fluid_keys: BTreeSet<String> = BTreeSet::new();
        let referenced: BTreeSet<String> = sorted_keys.iter().cloned().collect();
        let mut report = ParseReport::default();

        let items = build_items(
            &sorted_keys,
            &item_keys,
            &fluid_keys,
            &referenced,
            &[],
            &mut report,
        );

        assert_eq!(items.len(), 1);
        assert!(
            report.diagnostics.is_empty(),
            "a `{{`-shaped id is assumed synthetic, not unknown: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn with_namespace_adds_minecraft_only_where_the_namespace_is_missing() {
        assert_eq!(with_namespace("dirt"), "minecraft:dirt");
        assert_eq!(with_namespace("minecraft:dirt"), "minecraft:dirt");
        assert_eq!(with_namespace("chipped:dirt"), "chipped:dirt");
        // The `:` in an NBT suffix isn't a namespace.
        assert_eq!(
            with_namespace("programmed_circuit{configuration:1}"),
            "minecraft:programmed_circuit{configuration:1}"
        );
        assert_eq!(
            with_namespace("gtceu:programmed_circuit{configuration:1}"),
            "gtceu:programmed_circuit{configuration:1}"
        );
    }

    #[test]
    fn resolve_output_id_interns_any_string_unconditionally() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let first = resolve_output_id("gtceu:programmed_circuit{configuration:1}", &resolver);
        let second = resolve_output_id("gtceu:programmed_circuit{configuration:1}", &resolver);
        assert_eq!(first, second, "interning the same key is stable");

        let unrelated = resolve_output_id("minecraft:made_up_item", &resolver);
        assert_ne!(
            unrelated, first,
            "a different key gets a different (still provisional) index -- resolution never \
             fails here, whether or not the key is a known registered id"
        );
    }

    /// The property that makes this design correct: a recipe that only references real,
    /// registered items keeps the exact same final item ordering (alphabetical, across
    /// everything referenced or registered) regardless of what else a *different* recipe in
    /// the same export happens to reference, synthetic or not.
    #[test]
    fn real_items_sort_correctly_alongside_a_synthetic_id_discovered_elsewhere() {
        let files = [
            RawFile {
                path: "registries/item.json".to_owned(),
                bytes: br#"{"minecraft:dirt": "c", "minecraft:zombie_head": "c"}"#.to_vec(),
            },
            RawFile {
                path: "registries/fluid.json".to_owned(),
                bytes: b"{}".to_vec(),
            },
            RawFile {
                path: "recipes/gtceu/macerator/nbt_tool.json".to_owned(),
                bytes: br#"{
                    "type": "gtceu:macerator",
                    "duration": 100,
                    "tickInputs": {"eu_per_tick": [{"content": 8}]},
                    "inputs": {"item": [
                        {"content": {"type": "gtceu:sized", "count": 1, "ingredient": {
                            "type": "forge:nbt",
                            "item": "minecraft:egg",
                            "count": 1,
                            "nbt": "{CustomData:1}"
                        }}}
                    ]},
                    "outputs": {}
                }"#
                .to_vec(),
            },
            RawFile {
                path: "recipes/minecraft/zombie_head_from_dirt.json".to_owned(),
                bytes: br#"{
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [{"item": "minecraft:dirt"}],
                    "result": {"item": "minecraft:zombie_head"}
                }"#
                .to_vec(),
            },
        ];

        let (graph, report) =
            ingest(&files, DEFAULT_SHAPES_TOML).expect("DEFAULT_SHAPES_TOML is valid");

        assert!(
            report.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            report.diagnostics
        );

        // "minecraft:dirt" < "minecraft:egg{CustomData:1}" < "minecraft:zombie_head": the
        // synthetic id sorts *between* the two real ones, alphabetically.
        let item_ids: Vec<&str> = graph.items().iter().map(super::Item::id).collect();
        assert_eq!(
            item_ids,
            vec![
                "minecraft:dirt",
                "minecraft:egg{CustomData:1}",
                "minecraft:zombie_head",
            ],
        );

        let shapeless = graph
            .recipes()
            .iter()
            .find(|recipe| recipe.id() == "minecraft:zombie_head_from_dirt")
            .expect("the crafting_shapeless recipe ingested");
        let output = shapeless.outputs().first().expect("one output");
        assert_eq!(
            output.item(),
            ItemIndex::new(2),
            "minecraft:zombie_head sorts last overall, once the synthetic id is in the mix"
        );

        let macerator = graph
            .recipes()
            .iter()
            .find(|recipe| recipe.id() == "gtceu:macerator/nbt_tool")
            .expect("the macerator recipe ingested");
        let input = macerator.inputs().first().expect("one input");
        assert_eq!(
            input.members().iter().next().copied(),
            Some(ItemIndex::new(1)),
            "the NBT-folded synthetic id resolves to its final, correctly-sorted index"
        );
    }

    /// A tag gets resolved and its members interned regardless of whether any recipe uses it
    /// (see [`resolve_all_tags`]). That alone shouldn't count its members as referenced
    /// though.
    #[test]
    fn a_member_of_an_unused_tag_is_still_reported_as_registered_but_unreferenced() {
        let files = [
            RawFile {
                path: "registries/item.json".to_owned(),
                bytes: br#"{"minecraft:dirt": "c", "minecraft:stone": "c", "minecraft:planks": "c"}"#.to_vec(),
            },
            RawFile {
                path: "registries/fluid.json".to_owned(),
                bytes: b"{}".to_vec(),
            },
            RawFile {
                path: "tags/minecraft/item/forge/dirts.json".to_owned(),
                bytes: br#"["minecraft:dirt"]"#.to_vec(),
            },
            RawFile {
                path: "tags/minecraft/item/forge/stones.json".to_owned(),
                bytes: br#"["minecraft:stone"]"#.to_vec(),
            },
            RawFile {
                path: "recipes/minecraft/planks_from_dirt.json".to_owned(),
                bytes: br#"{
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [{"tag": "forge:dirts"}],
                    "result": {"item": "minecraft:planks"}
                }"#
                .to_vec(),
            },
        ];

        let (_graph, report) =
            ingest(&files, DEFAULT_SHAPES_TOML).expect("DEFAULT_SHAPES_TOML is valid");

        assert_eq!(
            report.registered_but_unreferenced,
            vec!["minecraft:stone".to_owned()],
            "forge:stones is never used by a recipe, so minecraft:dirt (via forge:dirts) and \
             minecraft:planks (a direct output) should be the only ids counted as referenced"
        );
    }
}
