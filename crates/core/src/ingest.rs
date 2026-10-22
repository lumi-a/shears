//! Turns a `/kubejs export` into a `model::Graph`.
//!
//! Problems turn into `Diagnostic`s in the returned `ParseReport`.
//!
//! Tags and the item/fluid registries are resolved first. Every recipe file can then be
//! parsed to real ids without incomplete drafts.

mod ingredient;
mod vanilla;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use self::ingredient::Leaf;
use crate::model::{Graph, Item, ItemIndex, ItemKind, Output, Recipe, Slot};

/// One file from the `/kubejs export`.
#[derive(Debug)]
pub struct RawFile {
    /// Path relative to the export root, separated by `/`, e.g.
    /// `recipes/minecraft/acacia_planks.json`.
    pub path: String,
    /// The file's raw bytes.
    pub bytes: Vec<u8>,
}

/// A problem noticed while ingesting.
#[derive(Debug)]
pub struct Diagnostic {
    /// A short label for whatever this concerns.
    ///
    /// Usually a file path, a recipe id, a tag id.
    ///
    /// This is not further typed, as it's meant to be read by the user.
    pub about: String,
    /// What went wrong.
    pub message: String,
}

/// Summary of what ingest did.
#[derive(Debug, Default)]
pub struct ParseReport {
    /// One diagnostic per problem.
    pub diagnostics: Vec<Diagnostic>,
    /// Number of recipe files seen, by their declared `type`.
    pub recipe_type_counts: BTreeMap<String, u32>,
}

impl ParseReport {
    /// Records one problem.
    fn fail(&mut self, about: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            about: about.into(),
            message: message.into(),
        });
    }
}

/// A recipe, as read from its JSON, already resolved to real ids.
///
/// The type is still a bare string, since the full set of types isn't known until every
/// recipe file has been seen.
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

/// The read-only lookups used while parsing the recipe files. This is built at the start
/// from the registries and tags.
struct Resolver<'a> {
    /// Every known item and fluid.
    item_index: &'a BTreeMap<String, ItemIndex>,
    /// Index into [`Graph::tags`] for the tags.
    tag_index: &'a BTreeMap<String, u32>,
    /// Resolved, transitively-expanded members for every tag in `tag_index`.
    tag_members: &'a BTreeMap<String, &'a BTreeSet<ItemIndex>>,
}

/// A per-recipe-type parsing function.
///
/// Inputs: Filepath, raw JSON, and resolver. Outputs: Draft xor diagnostic message.
type RecipeAdapter = fn(String, &Value, &Resolver<'_>) -> Result<RecipeDraft, String>;

/// Turns a raw export into a normalized graph and a parse report.
///
/// Vanilla recipe types only, for now; everything else becomes an "unhandled recipe type"
/// diagnostic rather than being silently dropped.
///
/// # Panics
///
/// Only if a single export has more than [`u32::MAX`] items, tags or recipe types, which
/// isn't a real export this tool will ever see.
#[must_use]
#[expect(
    clippy::missing_inline_in_public_items,
    reason = "top-level large function, usually has little reason to be inlined."
)]
pub fn ingest(files: &[RawFile]) -> (Graph, ParseReport) {
    let mut report = ParseReport::default();

    let (item_keys, fluid_keys, raw_tags) = read_registries_and_tags(files, &mut report);
    let (sorted_keys, item_index) = build_item_index(&item_keys, &fluid_keys);
    let (tags, tag_members_by_id) = resolve_all_tags(&raw_tags, &item_index, &mut report);
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
    let tag_members: BTreeMap<String, &BTreeSet<ItemIndex>> =
        tags.iter().cloned().zip(tag_members_by_id.iter()).collect();
    let items = build_items(sorted_keys, &tag_members_by_id);

    let resolver = Resolver {
        item_index: &item_index,
        tag_index: &tag_index,
        tag_members: &tag_members,
    };
    let mut recipe_drafts = parse_recipes(files, &resolver, &mut report);
    recipe_drafts.sort_unstable_by(|(_, a), (_, b)| a.id.cmp(&b.id));

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

    (Graph::new(1, items, recipes, tags, rtypes), report)
}

/// Reads `registries/item.json`, `registries/fluid.json`, and every item tag file
/// (`tags/minecraft/item/**`), returning raw members that aren't tag-resolved yet.
fn read_registries_and_tags(
    files: &[RawFile],
    report: &mut ParseReport,
) -> (
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeMap<String, Vec<String>>,
) {
    let mut item_keys = BTreeSet::new();
    let mut fluid_keys = BTreeSet::new();
    let mut raw_tags = BTreeMap::new();

    for file in files {
        match file.path.as_str() {
            "registries/item.json" => match parse_registry_keys(&file.bytes) {
                Ok(keys) => item_keys = keys,
                Err(message) => report.fail(&file.path, message),
            },
            "registries/fluid.json" => match parse_registry_keys(&file.bytes) {
                Ok(keys) => fluid_keys = keys,
                Err(message) => report.fail(&file.path, message),
            },
            path => {
                if let Some(tag_id) = item_tag_id(path) {
                    match parse_tag_members(&file.bytes) {
                        Ok(members) => {
                            let _: Option<Vec<String>> = raw_tags.insert(tag_id, members);
                        }
                        Err(message) => report.fail(&file.path, message),
                    }
                }
            }
        }
    }

    (item_keys, fluid_keys, raw_tags)
}

/// Parses a registry file (`{"id": "<java class>", ...}`), keeping only the ids.
fn parse_registry_keys(bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Ok(map.into_iter().map(|(key, _)| key).collect()),
        Ok(_) => Err("expected a JSON object".to_owned()),
        Err(error) => Err(format!("invalid JSON: {error}")),
    }
}

/// Parses a tag file.
///
/// These are flat arrays of member ids, where a `#`-prefix is another tag.
fn parse_tag_members(bytes: &[u8]) -> Result<Vec<String>, String> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Array(entries)) => entries
            .into_iter()
            .map(|entry| match entry {
                Value::String(member) => Ok(member),
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::Array(_)
                | Value::Object(_) => Err("tag member is not a string".to_owned()),
            })
            .collect(),
        Ok(_) => Err("expected a JSON array".to_owned()),
        Err(error) => Err(format!("invalid JSON: {error}")),
    }
}

/// Maps a raw file path under `tags/minecraft/item/` to the tag id it defines.
///
/// For instance, maps `tags/minecraft/item/forge/dusts.json` to `forge:dusts`.
fn item_tag_id(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix("tags/minecraft/item/")?
        .strip_suffix(".json")?;
    let (namespace, name) = rest.split_once('/')?;
    Some(format!("{namespace}:{name}"))
}

/// Maps a raw file path under `recipes/` to the recipe id it defines.
///
/// For instance, maps `recipes/minecraft/acacia_planks.json` to
/// `minecraft:acacia_planks`.
fn recipe_id_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("recipes/")?.strip_suffix(".json")?;
    let (namespace, name) = rest.split_once('/')?;
    Some(format!("{namespace}:{name}"))
}

/// Builds the sorted item/fluid list and its key-to-id lookup.
///
/// Doesn't depend on recipes or tags.
fn build_item_index(
    item_keys: &BTreeSet<String>,
    fluid_keys: &BTreeSet<String>,
) -> (Vec<(String, ItemKind)>, BTreeMap<String, ItemIndex>) {
    let mut all_keys: BTreeMap<String, ItemKind> = item_keys
        .iter()
        .cloned()
        .map(|key| (key, ItemKind::Item))
        .collect();
    for key in fluid_keys {
        let _: Option<ItemKind> = all_keys.insert(key.clone(), ItemKind::Fluid);
    }
    let sorted_keys: Vec<(String, ItemKind)> = all_keys.into_iter().collect();

    let item_index: BTreeMap<String, ItemIndex> = sorted_keys
        .iter()
        .enumerate()
        .map(|(index, (key, _))| {
            let index_u32 = u32::try_from(index).expect("fewer than u32::MAX items in one export. Please open an issue if this is a problem for you.");
            (key.clone(), ItemIndex::new(index_u32))
        })
        .collect();

    (sorted_keys, item_index)
}

/// Builds the final item list, now adding each item's tag membership.
///
/// `tag_members` is in the same order as the final `tags` list.
fn build_items(
    sorted_keys: Vec<(String, ItemKind)>,
    tag_members: &[BTreeSet<ItemIndex>],
) -> Vec<Item> {
    sorted_keys
        .into_iter()
        .enumerate()
        .map(|(index, (key, kind))| {
            let id =
                ItemIndex::new(u32::try_from(index).expect("fewer than u32::MAX items in one export. Please open an issue if this is a problem for you."));
            let item_tags: BTreeSet<u32> = tag_members
                .iter()
                .enumerate()
                .filter(|&(_, members)| members.contains(&id))
                .map(|(tag_index, _)| u32::try_from(tag_index).expect("fewer than u32::MAX tags. Please open an issue if this is a problem for you."))
                .collect();
            Item::new(key, kind, item_tags)
        })
        .collect()
}

/// Resolves a tag to its concrete, recursively-expanded members.
///
/// Memoizes into `resolved`. Also guards against a reference cycle and reports it in
/// `report`.
fn resolve_tag(
    id: &str,
    raw: &BTreeMap<String, Vec<String>>,
    resolved: &mut BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
    report: &mut ParseReport,
) -> BTreeSet<String> {
    if let Some(cached) = resolved.get(id) {
        return cached.clone();
    }
    if !visiting.insert(id.to_owned()) {
        report.fail(id, "cyclic tag reference");
        return BTreeSet::new();
    }

    let mut members = BTreeSet::new();
    match raw.get(id) {
        Some(raw_members) => {
            for entry in raw_members {
                if let Some(nested) = entry.strip_prefix('#') {
                    members.extend(resolve_tag(nested, raw, resolved, visiting, report));
                } else {
                    let _: bool = members.insert(entry.clone());
                }
            }
        }
        None => report.fail(id, "referenced tag has no tag file"),
    }

    let _: bool = visiting.remove(id);
    let _: Option<BTreeSet<String>> = resolved.insert(id.to_owned(), members.clone());
    members
}

/// Resolves every tag file found in the export to its concrete (interned) members.
///
/// A tag member that isn't a known item is reported in `report`.
///
/// Resolved (and interned) regardless of whether any recipe currently references it.
/// Unused tags really aren't our business.
fn resolve_all_tags(
    raw_tags: &BTreeMap<String, Vec<String>>,
    item_index: &BTreeMap<String, ItemIndex>,
    report: &mut ParseReport,
) -> (Vec<String>, Vec<BTreeSet<ItemIndex>>) {
    let mut resolved = BTreeMap::new();
    let mut tags = Vec::with_capacity(raw_tags.len());
    let mut tag_members = Vec::with_capacity(raw_tags.len());

    for tag_id in raw_tags.keys() {
        let mut visiting = BTreeSet::new();
        let string_members = resolve_tag(tag_id, raw_tags, &mut resolved, &mut visiting, report);
        let ids: BTreeSet<ItemIndex> = string_members
            .iter()
            .filter_map(|key| {
                if let Some(&id) = item_index.get(key) {
                    Some(id)
                } else {
                    report.fail(tag_id, format!("tag member {key:?} is not a known item"));
                    None
                }
            })
            .collect();
        tags.push(tag_id.clone());
        tag_members.push(ids);
    }

    (tags, tag_members)
}

/// Resolves a flat list of alternatives to the set of [`Item`]s.
fn resolve_leaves(leaves: &[Leaf], resolver: &Resolver<'_>) -> Result<BTreeSet<ItemIndex>, String> {
    let mut out = BTreeSet::new();
    for leaf in leaves {
        match leaf {
            Leaf::Item(key) => {
                let id = resolver
                    .item_index
                    .get(key.as_str())
                    .copied()
                    .ok_or_else(|| format!("unknown item {key:?}"))?;
                let _: bool = out.insert(id);
            }
            Leaf::Tag(tag_id) => {
                let members = resolver
                    .tag_members
                    .get(tag_id.as_str())
                    .ok_or_else(|| format!("unresolved tag {tag_id:?}"))?;
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
fn consumed_slot(leaves: &[Leaf], count: u32, resolver: &Resolver<'_>) -> Result<Slot, String> {
    let members = resolve_leaves(leaves, resolver)?;
    let tag = slot_tag(leaves, resolver);
    Ok(Slot::new(members, tag, count, 1.0))
}

/// How far outside `[0, 1]` a chance value may lie.
pub const CHANCE_TOLERANCE: f32 = 1e-5;

/// Validates a chance value read from the export, clamping it into `[0, 1]` if it's
/// only off by noise.
///
/// TODO: If the number is <= 0 + CHANCE_TOLERANCE or >= 1 - CHANCE_TOLERANCE, should we
/// also clamp it to 0 or 1 respectively? That would help with checks like "This is a
/// catalyst" and "this is a definitely consumed item", in case some recipes have, for any
/// reason, messed up numbers there.
///
/// TODO: This function should be used.
///
/// # Errors
///
/// If `raw` is more than [`CHANCE_TOLERANCE`] outside `[0, 1]`. That is, if `raw <=
/// -CHANCE_TOLERANCE` or `raw >= 1+CHANCE_TOLERANCE`.
#[inline]
pub fn validate_chance(raw: f32) -> Result<f32, String> {
    if raw < -CHANCE_TOLERANCE || raw > 1.0 + CHANCE_TOLERANCE {
        Err(format!("chance {raw} is out of range [0, 1]"))
    } else {
        Ok(raw.clamp(0.0, 1.0))
    }
}

/// Resolves a bare output item or fluid id to its [`ItemIndex`].
fn resolve_output_id(id: &str, resolver: &Resolver<'_>) -> Result<ItemIndex, String> {
    resolver
        .item_index
        .get(id)
        .copied()
        .ok_or_else(|| format!("unknown output item {id:?}"))
}

/// Parses every recognized recipe file, paired with its declared `type`.
///
/// Unhandled types are counted in the report but don't produce a draft.
///
/// TODO: Unhandled types should be handled and produce a draft, *AND* add a message to
/// the report complaining about the unknown type.
fn parse_recipes(
    files: &[RawFile],
    resolver: &Resolver<'_>,
    report: &mut ParseReport,
) -> Vec<(String, RecipeDraft)> {
    let mut drafts = Vec::new();

    for file in files {
        if !file.path.starts_with("recipes/") {
            continue;
        }
        let Some(id) = recipe_id_from_path(&file.path) else {
            report.fail(&file.path, "can't derive a recipe id from this path");
            continue;
        };

        let value: Value = match serde_json::from_slice(&file.bytes) {
            Ok(value) => value,
            Err(error) => {
                report.fail(&file.path, format!("invalid JSON: {error}"));
                continue;
            }
        };
        let Some(rtype) = value
            .get("type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            report.fail(&file.path, "missing \"type\"");
            continue;
        };
        *report.recipe_type_counts.entry(rtype.clone()).or_insert(0) += 1;

        let maybe_adapter: Option<RecipeAdapter> = match rtype.as_str() {
            "minecraft:crafting_shaped" => Some(vanilla::crafting_shaped),
            "minecraft:crafting_shapeless" => Some(vanilla::crafting_shapeless),
            "minecraft:smelting"
            | "minecraft:blasting"
            | "minecraft:smoking"
            | "minecraft:campfire_cooking" => Some(vanilla::cooking),
            "minecraft:stonecutting" => Some(vanilla::stonecutting),
            "minecraft:smithing_transform" => Some(vanilla::smithing_transform),
            _ => None,
        };

        let Some(adapter) = maybe_adapter else {
            report.fail(
                &file.path,
                format!("no adapter for recipe type {rtype:?} yet"),
            );
            continue;
        };

        match adapter(id, &value, resolver) {
            Ok(draft) => drafts.push((rtype, draft)),
            Err(message) => report.fail(&file.path, message),
        }
    }

    drafts
}

#[cfg(test)]
mod tests {
    use super::validate_chance;

    #[test]
    fn values_inside_the_range_pass_through_unchanged() {
        assert_eq!(validate_chance(0.0), Ok(0.0));
        assert_eq!(validate_chance(1.0), Ok(1.0));
        assert_eq!(validate_chance(0.5), Ok(0.5));
    }

    #[test]
    fn noise_within_tolerance_is_clamped_to_the_exact_boundary() {
        assert_eq!(validate_chance(-0.000001), Ok(0.0));
        assert_eq!(validate_chance(1.000001), Ok(1.0));
    }

    #[test]
    fn genuinely_out_of_range_is_an_error() {
        drop(validate_chance(-0.1).expect_err("-0.5 is far outside [0, 1]"));
        drop(validate_chance(1.1).expect_err("1.5 is far outside [0, 1]"));
    }
}
