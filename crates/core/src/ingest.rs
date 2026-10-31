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

mod ingredient;
mod path;
mod shapes;
mod vanilla;

use core::cell::RefCell;
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
    /// Every registered id that no recipe or tag ever references, sorted.
    pub registered_but_unreferenced: Vec<String>,
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
}

/// Interns every item/fluid id referenced while parsing a recipe (directly, or via a tag's
/// expanded members).
///
/// A plain registered id and a synthetic one (folded from a base id and an NBT string or
/// circuit configuration, see `shapes::fold_id_suffix`) are treated identically here.
///
/// None of this is final until every recipe file has been seen because that's the only point
/// at which the full set of referenced ids is known, which is needed to sort them
/// (determinism).So [`ItemInterner::intern`] hands out a provisional index for now, in
/// first-seen-order. [`ItemInterner::finish`] merges every interned id with every registered
/// one (so a registered-but-unreferenced id still gets a place in `Graph::items`), sorts the
/// result, and hands back a table remapping every provisional index to its final one.
#[derive(Debug, Default)]
struct ItemInterner {
    /// Every id referenced so far, to its provisional index.
    by_key: BTreeMap<String, ItemIndex>,
}

impl ItemInterner {
    /// Interns `key`, returning its provisional index. Stable across repeated calls with the
    /// same key.
    ///
    /// # Panics
    ///
    /// Only if a single export references more than [`u32::MAX`] distinct ids, which isn't a
    /// real export this tool will ever see.
    fn intern(&mut self, key: &str) -> ItemIndex {
        if let Some(&index) = self.by_key.get(key) {
            return index;
        }
        let index = ItemIndex::new(u32::try_from(self.by_key.len()).expect(
            "fewer than u32::MAX referenced ids in one export. \
            Please open an issue if this is a problem for you.",
        ));
        let _: Option<ItemIndex> = self.by_key.insert(key.to_owned(), index);
        index
    }

    /// Consumes every interned id, merges it with every id in `registered` (so a
    /// registered-but-unreferenced id still gets a place in the final set), and returns the
    /// merged, sorted key list plus a table mapping a provisional index to its final one.
    fn finish(
        self,
        registered: impl Iterator<Item = String>,
    ) -> (Vec<String>, Vec<ItemIndex>) {
        let mut final_keys: BTreeSet<String> = self.by_key.keys().cloned().collect();
        final_keys.extend(registered);
        let sorted_keys: Vec<String> = final_keys.into_iter().collect();

        let final_index_of: BTreeMap<&str, ItemIndex> = sorted_keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let index_u32 = u32::try_from(index).expect(
                    "fewer than u32::MAX items in one export. \
                    Please open an issue if this is a problem for you.",
                );
                (key.as_str(), ItemIndex::new(index_u32))
            })
            .collect();

        let mut remap = vec![ItemIndex::new(0); self.by_key.len()];
        for (key, provisional) in &self.by_key {
            let final_index = *final_index_of
                .get(key.as_str())
                .expect("every referenced key is included in final_keys by construction");
            *remap
                .get_mut(provisional.index())
                .expect("provisional indices are 0..by_key.len(), the exact size of remap") =
                final_index;
        }
        (sorted_keys, remap)
    }
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
/// Only if a single export has more than [`u32::MAX`] items, tags or recipe types, which isn't
/// a real export this tool will ever see.
#[must_use]
#[expect(
    clippy::missing_inline_in_public_items,
    reason = "top-level large function, usually has little reason to be inlined."
)]
pub fn ingest(files: &[RawFile]) -> (Graph, ParseReport) {
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

    let resolver = Resolver {
        tag_index: &tag_index,
        tag_members: &tag_members,
        interner: &interner_cell,
    };
    let mut recipe_drafts = parse_recipes(files, &resolver, &mut report);
    recipe_drafts.sort_unstable_by(|(_, a), (_, b)| a.id.cmp(&b.id));

    let interner = interner_cell.into_inner();
    let referenced: BTreeSet<String> = interner.by_key.keys().cloned().collect();
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

    (Graph::new(1, items, recipes, tags, rtypes), report)
}

/// Reads `registries/item.json`, `registries/fluid.json`, and every item and fluid tag file
/// (`tags/minecraft/item/**`, `tags/minecraft/fluid/**`), returning raw members that aren't
/// tag-resolved yet.
///
/// Item and fluid tags share one namespace here, same as `Slot`/`Item` don't otherwise
/// distinguish "an item tag" from "a fluid tag".
///
/// TODO: Should we distinguish the two, for sake of performance?
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
                if let Some(tag_id) = tag_id_from_path(path) {
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

/// Maps a raw file path under `tags/minecraft/item/` or `tags/minecraft/fluid/` to the tag id
/// it defines.
///
/// For instance, maps `tags/minecraft/item/forge/dusts.json` to `forge:dusts`, and
/// `tags/minecraft/fluid/forge/oxygen.json` to `forge:oxygen`.
fn tag_id_from_path(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix("tags/minecraft/item/")
        .or_else(|| path.strip_prefix("tags/minecraft/fluid/"))?
        .strip_suffix(".json")?;
    let (namespace, name) = rest.split_once('/')?;
    Some(format!("{namespace}:{name}"))
}

/// Maps a raw file path under `recipes/` to the recipe id it defines.
///
/// For instance, maps `recipes/minecraft/acacia_planks.json` to `minecraft:acacia_planks`.
fn recipe_id_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("recipes/")?.strip_suffix(".json")?;
    let (namespace, name) = rest.split_once('/')?;
    Some(format!("{namespace}:{name}"))
}

/// Builds the final `Item` list, in `sorted_keys`' order.
///
/// Classifies each key by checking `item_keys`/`fluid_keys`.
/// - Both means a registered item/fluid collision. This is reported, and the item wins.
/// - Neither means the key was only ever *referenced*, never registered. Either a synthetic
///   id, or an unknown-to-the-registers item, in which case it's reported.
/// - Also collects every registered id that's in neither `referenced` into
///   [`ParseReport::registered_but_unreferenced`]. These are items or fluids that no recipe
///   ever mentions. (TODO: Should probably not count tags as direct references, unless the tag
///   is actually referenced somewhere)
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
                    key,
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
                        key,
                        "referenced by a recipe, but not a registered item or fluid",
                    );
                }
                ItemKind::Item
            };
            Item::new(key.clone(), kind, item_tags)
        })
        .collect()
}

/// Rewrites every provisional [`ItemIndex`] in `drafts` to its final one, per `remap` (see
/// [`ItemInterner::finish`]).
fn remap_item_indices(drafts: &mut [(String, RecipeDraft)], remap: &[ItemIndex]) {
    for (_, draft) in drafts.iter_mut() {
        for slot in &mut draft.inputs {
            let members: BTreeSet<ItemIndex> = slot
                .members()
                .iter()
                .copied()
                .map(|id| remap_index(id, remap))
                .collect();
            *slot = Slot::new(members, slot.tag(), slot.count(), slot.chance());
        }
        for output in &mut draft.outputs {
            *output = Output::new(
                remap_index(output.item(), remap),
                output.count(),
                output.chance(),
            );
        }
    }
}

/// Looks up `id`'s final index in `remap` (see [`ItemInterner::finish`]).
fn remap_index(id: ItemIndex, remap: &[ItemIndex]) -> ItemIndex {
    remap
        .get(id.index())
        .copied()
        .expect("every id referenced while parsing was interned, so has an entry here")
}

/// Resolves a tag to its recursively-expanded members, as plain ids (not yet interned).
///
/// Memoizes into `resolved`. Also guards against a reference cycle and reports it in `report`.
///
/// A trailing `?` on an entry is Minecraft's own 'optional member' marker. Some mods write
/// `"modid:item?"` or `"#modid:tag?"` for a member that may not exist (e.g. when that mod
/// isn't loaded. This is silently skipped in-game, so we skip it silently too rather than
/// report it.
///
/// TODO: For all these strange suffixes like that, should we not at least also evaluate tags in the future against minecraft's identifier spec, to notice when there are strange ids going on? The spec in Java Edition is, taken from <https://minecraft.wiki/w/Identifier>:
///
/// - The namespace and the path may only contain `[.0-9a-z_-]`.
/// - The path (but not the namespace) may additionally contain a forward slash `/`.
/// - .. is not a valid namespace.
/// - `snake_case` is preferred.
fn resolve_tag(
    id: &str,
    raw: &BTreeMap<String, Vec<String>>,
    registered: &BTreeSet<&str>,
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
                let (core, optional) = entry
                    .strip_suffix('?')
                    .map_or((entry.as_str(), false), |stripped| (stripped, true));

                if let Some(nested) = core.strip_prefix('#') {
                    if !optional || raw.contains_key(nested) {
                        members.extend(resolve_tag(
                            nested, raw, registered, resolved, visiting, report,
                        ));
                    }
                } else if !optional || registered.contains(core) {
                    let _: bool = members.insert(core.to_owned());
                } else {
                    // Absent, but marked optional, so silently skip.
                }
            }
        }
        None => report.fail(id, "referenced tag has no tag file"),
    }

    let _: bool = visiting.remove(id);
    let _: Option<BTreeSet<String>> = resolved.insert(id.to_owned(), members.clone());
    members
}

/// Resolves every tag file found in the export to its members, interning each provisionally
/// (see [`ItemInterner`]).
///
/// Interning tag members here, once, rather than deferring them to plain ids that a recipe
/// would have to intern itself on every reference, is good for performance.
///
/// A required (non-`?`) tag member that isn't a known registered item or fluid is reported in
/// `report` and dropped from the result.
///
/// Tags resolved regardless of whether any recipe currently references it. Unused tags really
/// aren't our business.
fn resolve_all_tags(
    raw_tags: &BTreeMap<String, Vec<String>>,
    registered: &BTreeSet<&str>,
    interner: &RefCell<ItemInterner>,
    report: &mut ParseReport,
) -> (Vec<String>, Vec<BTreeSet<ItemIndex>>) {
    let mut resolved = BTreeMap::new();
    let mut tags = Vec::with_capacity(raw_tags.len());
    let mut tag_members = Vec::with_capacity(raw_tags.len());

    for tag_id in raw_tags.keys() {
        let mut visiting = BTreeSet::new();
        let string_members = resolve_tag(
            tag_id,
            raw_tags,
            registered,
            &mut resolved,
            &mut visiting,
            report,
        );
        let members: BTreeSet<ItemIndex> = string_members
            .into_iter()
            .filter_map(|key| {
                if registered.contains(key.as_str()) {
                    Some(interner.borrow_mut().intern(&key))
                } else {
                    report.fail(tag_id, format!("tag member {key:?} is not a known item"));
                    None
                }
            })
            .collect();
        tags.push(tag_id.clone());
        tag_members.push(members);
    }

    (tags, tag_members)
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
                let _: bool = out.insert(resolver.interner.borrow_mut().intern(key));
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

/// Validates a chance value read from the export, clamping it into `[0, 1]` if it's only off
/// by noise.
///
/// TODO: If the number is <= 0 + `CHANCE_TOLERANCE` or >= 1 - `CHANCE_TOLERANCE`, should we
/// also clamp it to 0 or 1 respectively? That would help with checks like "This is a catalyst"
/// and "this is a definitely consumed item", in case some recipes have, for any reason, messed
/// up numbers there.
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

/// Resolves a bare output item or fluid id to its provisional [`ItemIndex`]. Never fails,
/// see [`ItemInterner`].
fn resolve_output_id(id: &str, resolver: &Resolver<'_>) -> ItemIndex {
    resolver.interner.borrow_mut().intern(id)
}

/// Parses every recognized recipe file, paired with its declared `type`.
///
/// Unhandled types are counted in the report but don't produce a draft.
///
/// TODO: Unhandled types should be handled and produce a draft, *AND* add a message to the
/// report complaining about the unknown type.
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
            "minecraft:crafting_shaped"
            // GTCEu's own crafting-table-shaped types: structurally identical to
            // vanilla's (pattern/key/result), just registered under a different type id.
            | "gtceu:shaped"
            | "gtceu:crafting_shaped_strict"
            | "gtceu:crafting_shaped_energy_transfer"
            | "gtceu:crafting_shaped_fluid_container" => Some(vanilla::crafting_shaped),
            "minecraft:crafting_shapeless" => Some(vanilla::crafting_shapeless),
            "minecraft:smelting"
            | "minecraft:blasting"
            | "minecraft:smoking"
            | "minecraft:campfire_cooking" => Some(vanilla::cooking),
            "minecraft:stonecutting" => Some(vanilla::stonecutting),
            "minecraft:smithing_transform" => Some(vanilla::smithing_transform),
            _ => None,
        };

        if let Some(adapter) = maybe_adapter {
            match adapter(id, &value, resolver) {
                Ok(draft) => drafts.push((rtype, draft)),
                Err(message) => report.fail(&file.path, message),
            }
            continue;
        }

        match shapes::Shapes::builtin().shape_for(&rtype) {
            Some(shape) => match shapes::apply(id, &value, shape, resolver) {
                Ok(draft) => drafts.push((rtype, draft)),
                Err(message) => report.fail(&file.path, message),
            },
            None => report.fail(
                &file.path,
                format!("no adapter for recipe type {rtype:?} yet"),
            ),
        }
    }

    drafts
}

#[cfg(test)]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "these item ids deliberately contain `{...}` NBT/circuit-configuration \
              id_suffix suffixes, these are not format arguments"
)]
mod tests {
    use core::cell::RefCell;
    use core::iter;
    use std::collections::{BTreeMap, BTreeSet};

    use super::{
        build_items, ingest, resolve_all_tags, resolve_output_id, tag_id_from_path,
        validate_chance, ItemInterner, ParseReport, RawFile, Resolver,
    };
    use crate::model::{ItemIndex, ItemKind};

    #[test]
    fn values_inside_the_range_pass_through_unchanged() {
        assert_eq!(validate_chance(0.0), Ok(0.0));
        assert_eq!(validate_chance(1.0), Ok(1.0));
        assert_eq!(validate_chance(0.5), Ok(0.5));
    }

    #[test]
    fn noise_within_tolerance_is_clamped_to_the_exact_boundary() {
        assert_eq!(validate_chance(-0.000_001), Ok(0.0));
        assert_eq!(validate_chance(1.000_001), Ok(1.0));
    }

    #[test]
    fn genuinely_out_of_range_is_an_error() {
        drop(validate_chance(-0.1).expect_err("-0.5 is far outside [0, 1]"));
        drop(validate_chance(1.1).expect_err("1.5 is far outside [0, 1]"));
    }

    #[test]
    fn tag_id_from_path_accepts_both_item_and_fluid_tags() {
        assert_eq!(
            tag_id_from_path("tags/minecraft/item/forge/dusts.json"),
            Some("forge:dusts".to_owned())
        );
        assert_eq!(
            tag_id_from_path("tags/minecraft/fluid/forge/oxygen.json"),
            Some("forge:oxygen".to_owned())
        );
        assert_eq!(tag_id_from_path("recipes/minecraft/stick.json"), None);
    }

    #[test]
    fn an_absent_optional_plain_member_is_skipped_without_a_diagnostic() {
        let raw_tags =
            BTreeMap::from([("forge:milk".to_owned(), vec!["minecraft:milk?".to_owned()])]);
        let registered: BTreeSet<&str> = BTreeSet::new();
        let interner = RefCell::new(ItemInterner::default());
        let mut report = ParseReport::default();

        let (tags, tag_members) =
            resolve_all_tags(&raw_tags, &registered, &interner, &mut report);

        assert_eq!(tags, vec!["forge:milk".to_owned()]);
        assert!(tag_members.first().expect("one tag").is_empty());
        assert!(
            report.diagnostics.is_empty(),
            "an absent optional member shouldn't be reported: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn an_absent_required_plain_member_is_still_reported() {
        let raw_tags = BTreeMap::from([(
            "forge:dusts".to_owned(),
            vec!["minecraft:made_up_dust".to_owned()],
        )]);
        let registered: BTreeSet<&str> = BTreeSet::new();
        let interner = RefCell::new(ItemInterner::default());
        let mut report = ParseReport::default();

        let (_tags, tag_members) =
            resolve_all_tags(&raw_tags, &registered, &interner, &mut report);

        assert_eq!(report.diagnostics.len(), 1);
        assert!(
            tag_members.first().expect("one tag").is_empty(),
            "the unresolved required member is dropped from the tag's members"
        );
    }

    #[test]
    fn an_absent_optional_nested_tag_is_skipped_without_a_diagnostic() {
        let raw_tags = BTreeMap::from([(
            "forge:fuel".to_owned(),
            vec!["#forge:biodiesel?".to_owned()],
        )]);
        let registered: BTreeSet<&str> = BTreeSet::new();
        let interner = RefCell::new(ItemInterner::default());
        let mut report = ParseReport::default();

        let (_tags, tag_members) =
            resolve_all_tags(&raw_tags, &registered, &interner, &mut report);

        assert!(tag_members.first().expect("one tag").is_empty());
        assert!(
            report.diagnostics.is_empty(),
            "an absent optional nested tag shouldn't be reported: {:?}",
            report.diagnostics
        );
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
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("both an item and a fluid")));
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
            diagnostic
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
    fn item_interner_assigns_final_indices_in_sorted_order_regardless_of_discovery_order() {
        let mut interner = ItemInterner::default();

        // Discovered in this order, but "a_first" sorts before "z_last".
        let z_last_provisional = interner.intern("z_last");
        let a_first_provisional = interner.intern("a_first");
        assert_eq!(z_last_provisional, ItemIndex::new(0), "first discovered");
        assert_eq!(a_first_provisional, ItemIndex::new(1));
        assert_eq!(
            interner.intern("z_last"),
            z_last_provisional,
            "interning again is stable"
        );

        let (sorted_keys, remap) = interner.finish(iter::empty());

        assert_eq!(sorted_keys, vec!["a_first".to_owned(), "z_last".to_owned()]);
        // "z_last"'s provisional index was 0.
        assert_eq!(
            remap.first().copied(),
            Some(ItemIndex::new(1)),
            "z_last sorts second, so index 1"
        );
        // "a_first"'s provisional index was 1.
        assert_eq!(
            remap.get(1).copied(),
            Some(ItemIndex::new(0)),
            "a_first sorts first, so index 0"
        );
    }

    #[test]
    fn item_interner_finish_includes_a_registered_but_unreferenced_id() {
        let mut interner = ItemInterner::default();
        let _provisional = interner.intern("minecraft:referenced");

        let (sorted_keys, _remap) =
            interner.finish(iter::once("minecraft:unreferenced".to_owned()));

        assert_eq!(
            sorted_keys,
            vec![
                "minecraft:referenced".to_owned(),
                "minecraft:unreferenced".to_owned(),
            ]
        );
    }

    #[test]
    fn resolve_output_id_interns_any_string_unconditionally() {
        let tag_index: BTreeMap<String, u32> = BTreeMap::new();
        let tag_members: BTreeMap<String, &BTreeSet<ItemIndex>> = BTreeMap::new();
        let interner = RefCell::new(ItemInterner::default());
        let resolver = Resolver {
            tag_index: &tag_index,
            tag_members: &tag_members,
            interner: &interner,
        };

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

        let (graph, report) = ingest(&files);

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
}
