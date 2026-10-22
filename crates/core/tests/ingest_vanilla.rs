//! Ingests the committed vanilla fixture and checks the shape of the result.

#![expect(
    unused_crate_dependencies,
    reason = "this integration test only exercises ingest/io through shears_core's \
              public API, not every dependency the crate declares."
)]

use std::fs::File;
use std::path::{Path, PathBuf};

fn extract_fixture(destination: &Path) {
    // Is this relative path stuff the correct way of doing things?
    let archive_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vanilla-1.20.1.tar.zst");
    let archive = File::open(&archive_path).expect("the committed vanilla fixture should exist");
    let decoder = zstd::Decoder::new(archive).expect("a valid zstd stream");
    tar::Archive::new(decoder)
        .unpack(destination)
        .expect("a valid tar archive");
}

#[cfg(test)]
mod tests {
    use super::extract_fixture;
    use shears_core::{ingest, io};

    #[test]
    fn ingests_the_vanilla_fixture_with_only_expected_gaps() {
        let scratch = tempfile::tempdir().expect("a scratch directory");
        extract_fixture(scratch.path());
        let root = scratch.path().join("vanilla-1.20.1");

        let files = io::read_export_dir(&root).expect("reading the extracted export");
        let (graph, report) = ingest::ingest(&files);

        let graph_json = serde_json::to_value(&graph).expect("serializing the ingested graph");
        let items = graph_json
            .get("items")
            .and_then(|v| v.as_array())
            .expect("an items array");
        let recipes = graph_json
            .get("recipes")
            .and_then(|v| v.as_array())
            .expect("a recipes array");

        // Every recipe type without an adapter yet:
        // - `crafting_special_*` (13 files)
        // - `crafting_decorated_pot` (1)
        // - `smithing_trim` (16)
        //
        // If this count changes, check whether it's a new unhandled type or a real
        // regression before updating it.
        let unhandled_type_diagnostics = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("no adapter for recipe type"))
            .count();
        assert_eq!(
            unhandled_type_diagnostics, 30,
            "unexpected set of unhandled recipe types"
        );
        assert_eq!(
            report.diagnostics.len(),
            unhandled_type_diagnostics,
            "a handled recipe type produced a diagnostic: {:?}",
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| !diagnostic.message.contains("no adapter for recipe type"))
                .collect::<Vec<_>>()
        );

        // 1174 recipe files total, minus the 30 unhandled above.
        assert_eq!(recipes.len(), 1174 - 30);

        // Every registered item and fluid, whether or not any recipe mentions it.
        assert!(
            items.len() > 1000,
            "expected over a thousand vanilla items and fluids, got {}",
            items.len()
        );

        let acacia_planks = items
            .iter()
            .find(|item| {
                item.get("id")
                    .is_some_and(|id| id == "minecraft:acacia_planks")
            })
            .expect("minecraft:acacia_planks should be a known item");
        assert_eq!(
            acacia_planks.get("kind").and_then(|v| v.as_str()),
            Some("Item")
        );

        let smelting_recipe = recipes
            .iter()
            .find(|recipe| {
                recipe
                    .get("id")
                    .is_some_and(|id| id == "minecraft:iron_ingot_from_smelting_iron_ore")
            })
            .expect("smelting iron_ingot from iron_ore should have ingested");
        let outputs = smelting_recipe
            .get("outputs")
            .and_then(|v| v.as_array())
            .expect("an outputs array");
        assert_eq!(outputs.len(), 1);
    }
}
