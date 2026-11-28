//! Reading recipe files into drafts.

use serde_json::Value;

use super::{
    About, ParseReport, RawFile, RecipeDraft, Resolver, recipe_id_from_path, shapes, vanilla,
};

/// A per-recipe-type parsing function.
///
/// Inputs: Filepath, raw JSON, and resolver. Outputs: Draft xor diagnostic message.
type RecipeAdapter = fn(String, &Value, &Resolver<'_>) -> Result<RecipeDraft, String>;

/// Parses every recognized recipe file, paired with its declared `type`.
///
/// Unhandled types are counted in the report but don't produce a draft.
///
/// TODO: Unhandled types should be handled and produce a draft, *AND* add a message to the
/// report complaining about the unknown type.
pub(super) fn parse_recipes(
    files: &[RawFile],
    resolver: &Resolver<'_>,
    shapes: &shapes::Shapes,
    report: &mut ParseReport,
) -> Vec<(String, RecipeDraft)> {
    let mut drafts = Vec::new();

    for file in files {
        if !file.path.starts_with("recipes/") {
            continue;
        }
        let Some(id) = recipe_id_from_path(&file.path) else {
            report.fail(
                About::file(&file.path),
                "can't derive a recipe id from this path",
            );
            continue;
        };

        let value: Value = match serde_json::from_slice(&file.bytes) {
            Ok(value) => value,
            Err(error) => {
                report.fail(About::file(&file.path), format!("invalid JSON: {error}"));
                continue;
            }
        };
        let Some(rtype) = value
            .get("type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            report.fail(About::file(&file.path), "missing \"type\"");
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
                Err(message) => report.fail(About::file(&file.path), message),
            }
            continue;
        }

        match shapes.shape_for(&rtype) {
            Some(shape) => match shapes::apply(id, &value, shape, resolver) {
                Ok(draft) => drafts.push((rtype, draft)),
                Err(message) => report.fail(About::file(&file.path), message),
            },
            None => report.fail(
                About::file(&file.path),
                format!("no adapter for recipe type {rtype:?} yet"),
            ),
        }
    }

    drafts
}
