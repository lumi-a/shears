//! Reading the registries and tag files, and resolving tags to their members.

use core::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{About, ItemInterner, ParseReport, RawFile, with_namespace};
use crate::model::ItemIndex;

/// Reads `registries/item.json`, `registries/fluid.json`, and every item and fluid tag file
/// (`tags/minecraft/item/**`, `tags/minecraft/fluid/**`), returning raw members that aren't
/// tag-resolved yet.
///
/// Item and fluid tags share one namespace here, same as `Slot`/`Item` don't otherwise
/// distinguish "an item tag" from "a fluid tag".
///
/// TODO: Should we distinguish the two, for sake of performance?
pub(super) fn read_registries_and_tags(
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
                Err(message) => report.fail(About::file(&file.path), message),
            },
            "registries/fluid.json" => match parse_registry_keys(&file.bytes) {
                Ok(keys) => fluid_keys = keys,
                Err(message) => report.fail(About::file(&file.path), message),
            },
            path => {
                if let Some(tag_id) = tag_id_from_path(path) {
                    match parse_tag_members(&file.bytes) {
                        Ok(members) => {
                            let _: Option<Vec<String>> = raw_tags.insert(tag_id, members);
                        }
                        Err(message) => report.fail(About::file(&file.path), message),
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
/// These are flat arrays of member ids, where a `#`-prefix is another tag. Every id gets
/// [`with_namespace`].
pub(super) fn parse_tag_members(bytes: &[u8]) -> Result<Vec<String>, String> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Array(entries)) => entries
            .into_iter()
            .map(|entry| match entry {
                Value::String(member) => Ok(member.strip_prefix('#').map_or_else(
                    || with_namespace(&member).into_owned(),
                    |tag| format!("#{}", with_namespace(tag)),
                )),
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
pub(super) fn tag_id_from_path(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix("tags/minecraft/item/")
        .or_else(|| path.strip_prefix("tags/minecraft/fluid/"))?
        .strip_suffix(".json")?;
    let (namespace, name) = rest.split_once('/')?;
    Some(format!("{namespace}:{name}"))
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
        report.fail(About::tag(id), "cyclic tag reference");
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
        None => report.fail(About::tag(id), "referenced tag has no tag file"),
    }

    let _: bool = visiting.remove(id);
    let _: Option<BTreeSet<String>> = resolved.insert(id.to_owned(), members.clone());
    members
}

/// Resolves every tag file found in the export to its members, interning each provisionally
/// (see [`ItemInterner`]).
///
/// Interning tag members here immediately instead of keeping them as plain ids that a recipe
/// would have to intern itself on every reference, is good for performance.
///
/// A required (non-`?`) tag member that isn't a known registered item or fluid is reported in
/// `report` and dropped from the result.
///
/// Tags resolved regardless of whether any recipe currently references it. Unused tags really
/// aren't our business.
pub(super) fn resolve_all_tags(
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
                    report.fail(
                        About::tag(tag_id),
                        format!("tag member {key:?} is not a known item"),
                    );
                    None
                }
            })
            .collect();
        tags.push(tag_id.clone());
        tag_members.push(members);
    }

    (tags, tag_members)
}
