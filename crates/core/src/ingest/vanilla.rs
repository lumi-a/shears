//! Adapters for vanilla recipe types like crafting, cooking, stonecutting, smithing.

use std::collections::BTreeMap;

use serde_json::Value;

use super::ingredient::parse_ingredient;
use super::{consumed_slot, resolve_output_id, RecipeDraft, Resolver};
use crate::model::Output;

/// Reads a JSON number as `u32`.
///
/// Rejects non-integer numbers.
fn as_u32(value: &Value) -> Option<u32> {
    let count = value.as_u64()?;
    u32::try_from(count).ok()
}

/// Parses and resolves the `{"item": ..., "count": ...}` result shape that's used by crafting
/// and smithing.
fn parse_result_object(value: &Value, resolver: &Resolver<'_>) -> Result<Output, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "`result` is not an object".to_owned())?;
    let id = object
        .get("item")
        .and_then(Value::as_str)
        .ok_or_else(|| "`result` is an object but has no key `item`".to_owned())?;
    let count = match object.get("count") {
        None => 1,
        Some(count_value) => as_u32(count_value)
            .ok_or_else(|| "`result.count` exists but is not a number".to_owned())?,
    };
    let item = resolve_output_id(id, resolver);
    Ok(Output::new(item, count, 1.0))
}

/// Parses and resolves a `result` that's either a bare item id (count 1, vanilla's own
/// convention for cooking) or an object `{item, count}` (some cooking recipes from other mods
/// use this instead, with the same meaning).
fn parse_result(value: &Value, resolver: &Resolver<'_>) -> Result<Output, String> {
    match value {
        Value::String(id) => {
            let item = resolve_output_id(id, resolver);
            Ok(Output::new(item, 1, 1.0))
        }
        Value::Object(_) => parse_result_object(value, resolver),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
            Err("`result` is neither a bare item id nor an object".to_owned())
        }
    }
}

/// `minecraft:crafting_shaped` has a `key` of symbols to ingredient and a `pattern` of
/// symbols. The symbol count is how often it appears across the pattern.
pub(super) fn crafting_shaped(
    id: String,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<RecipeDraft, String> {
    let key = value
        .get("key")
        .and_then(Value::as_object)
        .ok_or_else(|| "missing or malformed `key`".to_owned())?;
    let pattern = value
        .get("pattern")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing or malformed `pattern`".to_owned())?;

    // Pattern rows and key symbols are matched by KubeJS/Java, whose strings are
    // UTF-16: a Java `char` is one UTF-16 code unit, not one Rust `char` (one Unicode
    // scalar value). They agree for every symbol in the Basic Multilingual Plane
    // (everything realistically used), but a symbol above it is one Rust `char` and
    // two Java `char`s, i.e. two grid cells there, one here if we iterated by Rust
    // `char`. Iterating `encode_utf16()` instead matches Java's own semantics,
    // not just for every symbol seen so far.
    let mut unit_counts: BTreeMap<u16, u32> = BTreeMap::new();
    for row_value in pattern {
        let row_str = row_value
            .as_str()
            .ok_or_else(|| "the pattern's row is not a string".to_owned())?;
        for symbol in row_str
            .encode_utf16()
            .filter(|&symbol| symbol != u16::from(b' '))
        {
            *unit_counts.entry(symbol).or_insert(0) += 1;
        }
    }

    let mut inputs = Vec::new();
    // Try and decypher the key
    // We do this *after* parsing the pattern, so that we can skip having
    // to worry about unused keys (see `continue` inside)
    for (symbol, ingredient_value) in key {
        let mut units = symbol.encode_utf16();
        let Some(symbol_unit) = units.next() else {
            return Err("empty key symbol".to_owned());
        };
        if units.next().is_some() {
            return Err(format!("multi-character key symbol {symbol:?}"));
        }
        let Some(&cell_count) = unit_counts.get(&symbol_unit) else {
            // Declared in `key` but never used in `pattern`.
            // Not our business.
            continue;
        };
        let leaves = parse_ingredient(ingredient_value)
            .ok_or_else(|| format!("unrecognized ingredient shape for key {symbol:?}"))?;
        let slot = consumed_slot(&leaves, cell_count, resolver)
            .map_err(|error| format!("key {symbol:?}: {error}"))?;
        inputs.push(slot);
    }

    let result = value
        .get("result")
        .ok_or_else(|| "missing `result`".to_owned())?;
    let output = parse_result_object(result, resolver)?;

    Ok(RecipeDraft {
        id,
        inputs,
        outputs: vec![output],
        // TODO:
        // There *are* automation machines that can craft stuff
        // for you. Same for stonecutting and smithing.
        // But I don't yet know how to properly accept that
        // information (energy, time) from the user; surely this
        // won't be in KubeJS's output.
        eu_t: None,
        ticks: None,
    })
}

/// `minecraft:crafting_shapeless`: an `ingredients` array.
///
/// Repeated identical entries are merged and added into one slot.
pub(super) fn crafting_shapeless(
    id: String,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<RecipeDraft, String> {
    let ingredients = value
        .get("ingredients")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing or malformed `ingredients`".to_owned())?;

    let mut merged: Vec<(Vec<super::ingredient::Leaf>, u32)> = Vec::new();
    for ingredient_value in ingredients {
        let leaves = parse_ingredient(ingredient_value)
            .ok_or_else(|| "unrecognized ingredient shape in `ingredients`".to_owned())?;
        if let Some(existing) = merged.iter_mut().find(|(existing, _)| *existing == leaves) {
            existing.1 += 1;
        } else {
            merged.push((leaves, 1));
        }
    }

    let mut inputs = Vec::with_capacity(merged.len());
    for (leaves, count) in merged {
        inputs.push(consumed_slot(&leaves, count, resolver)?);
    }

    let result = value
        .get("result")
        .ok_or_else(|| "missing `result`".to_owned())?;
    let output = parse_result_object(result, resolver)?;

    Ok(RecipeDraft {
        id,
        inputs,
        outputs: vec![output],
        eu_t: None,
        ticks: None,
    })
}

/// `minecraft:smelting`, `blasting`, `smoking`, `campfire_cooking`.
///
/// A single `ingredient`, a `result`, and a `cookingtime`.
pub(super) fn cooking(
    id: String,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<RecipeDraft, String> {
    let ingredient_value = value
        .get("ingredient")
        .ok_or_else(|| "missing `ingredient`".to_owned())?;
    let leaves = parse_ingredient(ingredient_value)
        .ok_or_else(|| "unrecognized ingredient shape in `ingredient`".to_owned())?;
    let input = consumed_slot(&leaves, 1, resolver)?;

    let result = value
        .get("result")
        .ok_or_else(|| "missing `result`".to_owned())?;
    let output = parse_result(result, resolver)?;

    let ticks = match value.get("cookingtime") {
        None => None,
        Some(cookingtime_value) => Some(
            cookingtime_value
                .as_u64()
                .and_then(|ticks| u32::try_from(ticks).ok())
                .ok_or_else(|| "`cookingtime` is not a u32".to_owned())?,
        ),
    };

    Ok(RecipeDraft {
        id,
        inputs: vec![input],
        outputs: vec![output],
        eu_t: None,
        ticks,
    })
}

/// `minecraft:stonecutting` has a single `ingredient`, a `count`, and a `result` that's a bare
/// item id.
pub(super) fn stonecutting(
    id: String,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<RecipeDraft, String> {
    let ingredient_value = value
        .get("ingredient")
        .ok_or_else(|| "missing `ingredient`".to_owned())?;
    let leaves = parse_ingredient(ingredient_value)
        .ok_or_else(|| "unrecognized ingredient shape in `ingredient`".to_owned())?;
    let input = consumed_slot(&leaves, 1, resolver)?;

    let result = value
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| "`result` is not a bare item id".to_owned())?;
    let output_item = resolve_output_id(result, resolver);
    let count = match value.get("count") {
        None => 1,
        Some(count_value) => {
            as_u32(count_value).ok_or_else(|| "`count` is not a number".to_owned())?
        }
    };

    Ok(RecipeDraft {
        id,
        inputs: vec![input],
        outputs: vec![Output::new(output_item, count, 1.0)],
        eu_t: None,
        ticks: None,
    })
}

/// `minecraft:smithing_transform` has a `template`, `base` and `addition` as ingredients, and
/// an object `result`.
pub(super) fn smithing_transform(
    id: String,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<RecipeDraft, String> {
    let mut inputs = Vec::new();
    for key in ["template", "base", "addition"] {
        let ingredient_value = value.get(key).ok_or_else(|| format!("missing {key:?}"))?;
        let leaves = parse_ingredient(ingredient_value)
            .ok_or_else(|| format!("unrecognized ingredient shape in {key:?}"))?;
        inputs.push(
            consumed_slot(&leaves, 1, resolver)
                .map_err(|error| format!("{key:?}: {error}"))?,
        );
    }

    let result = value
        .get("result")
        .ok_or_else(|| "missing `result`".to_owned())?;
    let output = parse_result_object(result, resolver)?;

    Ok(RecipeDraft {
        id,
        inputs,
        outputs: vec![output],
        eu_t: None,
        ticks: None,
    })
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::{
        cooking, crafting_shaped, crafting_shapeless, smithing_transform, stonecutting,
    };
    use crate::ingest::{ItemInterner, PlainResolverParts, Resolver};
    use crate::model::ItemIndex;

    #[test]
    fn crafting_shaped_counts_pattern_occurrences() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let draft = crafting_shaped(
            "minecraft:acacia_trapdoor".to_owned(),
            &json!({
                "type": "minecraft:crafting_shaped",
                "key": {"#": {"item": "minecraft:acacia_planks"}},
                "pattern": ["###", "###"],
                "result": {"count": 2, "item": "minecraft:acacia_trapdoor"},
            }),
            &resolver,
        )
        .expect("a well-formed crafting_shaped recipe");

        assert_eq!(draft.inputs.len(), 1);
        assert_eq!(draft.inputs.first().expect("one input slot").count(), 6);
        let output = draft.outputs.first().expect("one output");
        let expected = parts
            .interner
            .borrow_mut()
            .intern("minecraft:acacia_trapdoor");
        assert_eq!(output.item(), expected);
        assert_eq!(output.count(), 2);
    }

    #[test]
    fn crafting_shaped_rejects_a_key_symbol_above_the_basic_multilingual_plane() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        // U+1F642 is one Rust `char` but two UTF-16 code units (a surrogate pair).
        // Two Java `char`s, so KubeJS/Java would never accept it as a single pattern
        // symbol either. Confirms `encode_utf16` rejects it instead of accepting it
        // as a single symbol the way iterating by Rust `char` would have.
        let error = crafting_shaped(
            "minecraft:acacia_trapdoor".to_owned(),
            &json!({
                "type": "minecraft:crafting_shaped",
                "key": {"🙂": {"item": "minecraft:acacia_planks"}},
                "pattern": ["🙂🙂🙂", "🙂🙂🙂"],
                "result": {"count": 2, "item": "minecraft:acacia_trapdoor"},
            }),
            &resolver,
        )
        .expect_err("a key symbol above the BMP should be rejected, not accepted as one unit");

        assert!(
            error.contains("multi-character key symbol"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn crafting_shaped_defaults_missing_result_count_to_one() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let draft = crafting_shaped(
            "minecraft:acacia_boat".to_owned(),
            &json!({
                "type": "minecraft:crafting_shaped",
                "key": {"#": {"item": "minecraft:acacia_planks"}},
                "pattern": ["# #", "###"],
                "result": {"item": "minecraft:acacia_boat"},
            }),
            &resolver,
        )
        .expect("a well-formed crafting_shaped recipe");

        assert_eq!(draft.outputs.first().expect("one output").count(), 1);
    }

    #[test]
    fn crafting_shapeless_merges_duplicate_ingredients_into_one_slot() {
        let tag_index: BTreeMap<String, u32> =
            BTreeMap::from([("minecraft:acacia_logs".to_owned(), 0)]);
        let acacia_logs: BTreeSet<ItemIndex> = BTreeSet::from([ItemIndex::new(0)]);
        let tag_members: BTreeMap<String, &BTreeSet<ItemIndex>> =
            BTreeMap::from([("minecraft:acacia_logs".to_owned(), &acacia_logs)]);
        let interner = RefCell::new(ItemInterner::default());
        let direct_refs = RefCell::new(BTreeSet::new());
        let used_tags = RefCell::new(BTreeSet::new());
        let resolver = Resolver {
            tag_index: &tag_index,
            tag_members: &tag_members,
            interner: &interner,
            direct_refs: &direct_refs,
            used_tags: &used_tags,
        };

        let draft = crafting_shapeless(
            "minecraft:acacia_planks".to_owned(),
            &json!({
                "type": "minecraft:crafting_shapeless",
                "ingredients": [{"tag": "minecraft:acacia_logs"}],
                "result": {"count": 4, "item": "minecraft:acacia_planks"},
            }),
            &resolver,
        )
        .expect("a well-formed crafting_shapeless recipe");

        assert_eq!(draft.inputs.len(), 1);
        assert_eq!(draft.inputs.first().expect("one input slot").count(), 1);
        assert_eq!(draft.inputs.first().expect("one input slot").tag(), Some(0));
    }

    #[test]
    fn cooking_reads_a_bare_string_result_and_cookingtime() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let draft = cooking(
            "minecraft:baked_potato".to_owned(),
            &json!({
                "type": "minecraft:smelting",
                "cookingtime": 200,
                "ingredient": {"item": "minecraft:potato"},
                "result": "minecraft:baked_potato",
            }),
            &resolver,
        )
        .expect("a well-formed cooking recipe");

        let output = draft.outputs.first().expect("one output");
        let expected = parts.interner.borrow_mut().intern("minecraft:baked_potato");
        assert_eq!(output.item(), expected);
        assert_eq!(draft.ticks, Some(200));
    }

    #[test]
    fn cooking_also_reads_an_object_result_with_a_count() {
        // GTCEu smelting recipes commonly use the object form instead of
        // vanilla's bare strings.
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let draft = cooking(
            "projectred_core:infused_silicon".to_owned(),
            &json!({
                "type": "minecraft:smelting",
                "cookingtime": 200,
                "ingredient": {"item": "projectred_core:red_silicon_comp"},
                "result": {"item": "projectred_core:infused_silicon", "count": 2},
            }),
            &resolver,
        )
        .expect("a well-formed cooking recipe with an object result");

        let output = draft.outputs.first().expect("one output");
        let expected = parts
            .interner
            .borrow_mut()
            .intern("projectred_core:infused_silicon");
        assert_eq!(output.item(), expected);
        assert_eq!(output.count(), 2);
    }

    #[test]
    fn stonecutting_applies_count_to_the_output() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let draft = stonecutting(
            "minecraft:andesite_slab_from_andesite_stonecutting".to_owned(),
            &json!({
                "type": "minecraft:stonecutting",
                "count": 2,
                "ingredient": {"item": "minecraft:andesite"},
                "result": "minecraft:andesite_slab",
            }),
            &resolver,
        )
        .expect("a well-formed stonecutting recipe");

        assert_eq!(draft.outputs.first().expect("one output").count(), 2);
    }

    #[test]
    fn smithing_transform_reads_all_three_input_slots() {
        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let draft = smithing_transform(
            "minecraft:netherite_leggings_smithing".to_owned(),
            &json!({
                "type": "minecraft:smithing_transform",
                "template": {"item": "minecraft:netherite_upgrade_smithing_template"},
                "base": {"item": "minecraft:diamond_leggings"},
                "addition": {"item": "minecraft:netherite_ingot"},
                "result": {"item": "minecraft:netherite_leggings"},
            }),
            &resolver,
        )
        .expect("a well-formed smithing_transform recipe");

        assert_eq!(draft.inputs.len(), 3);
        let output = draft.outputs.first().expect("one output");
        let expected = parts
            .interner
            .borrow_mut()
            .intern("minecraft:netherite_leggings");
        assert_eq!(output.item(), expected);
    }
}
