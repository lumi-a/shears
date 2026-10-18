//! Parses the ingredient JSON shapes recipe types use across mods.

use serde_json::Value;

/// A single concrete alternative in an ingredient's OR-list.
///
/// Either singleton, or tag-set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Leaf {
    /// A single item or fluid, by id.
    Item(String),
    /// A set of items or fluids, represented by a tag id.
    Tag(String),
}

/// Parses a single alternative bare item or tag.
///
/// Didn't see an array nested inside so far, so for now we treat
/// it as unrecognised instead of flattening it.
fn parse_leaf(value: &Value) -> Option<Leaf> {
    match value {
        Value::Object(map) => match (map.get("item"), map.get("tag")) {
            (Some(Value::String(item)), _) => Some(Leaf::Item(item.clone())),
            (_, Some(Value::String(tag))) => Some(Leaf::Tag(tag.clone())),
            _ => None,
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            None
        }
    }
}

/// Parses an ingredient into a list of alternatives (an OR).
///
/// Bare items or tags become a singleton lists. Arrays become a list.
///
/// Returns `None` for shapes this doesn't recognize.
pub(super) fn parse_ingredient(value: &Value) -> Option<Vec<Leaf>> {
    match value {
        Value::Array(entries) => entries.iter().map(parse_leaf).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            parse_leaf(value).map(|leaf| vec![leaf])
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Leaf, parse_ingredient};

    #[test]
    fn parses_a_bare_item_as_a_singleton() {
        assert_eq!(
            parse_ingredient(&json!({"item": "minecraft:stick"})),
            Some(vec![Leaf::Item("minecraft:stick".to_owned())]),
        );
    }

    #[test]
    fn parses_a_bare_tag_as_a_singleton() {
        assert_eq!(
            parse_ingredient(&json!({"tag": "minecraft:planks"})),
            Some(vec![Leaf::Tag("minecraft:planks".to_owned())]),
        );
    }

    #[test]
    fn parses_a_flat_array_of_alternatives() {
        // The real shape, e.g. dye_blue_bed.json's bed-color ingredient: a flat list
        // of items, one array nesting level, no leaf is itself an array.
        assert_eq!(
            parse_ingredient(&json!([
                {"item": "minecraft:red_bed"},
                {"item": "minecraft:green_bed"},
            ])),
            Some(vec![
                Leaf::Item("minecraft:red_bed".to_owned()),
                Leaf::Item("minecraft:green_bed".to_owned()),
            ]),
        );
    }

    #[test]
    fn rejects_an_array_nested_inside_an_array() {
        // Hasn't been seen in any real recipe. Want to make sure
        // it shows up as a diagnostic
        // if that ever happens.
        assert_eq!(
            parse_ingredient(&json!([
                {"item": "minecraft:blue_dye"},
                [{"item": "minecraft:red_bed"}, {"item": "minecraft:green_bed"}],
            ])),
            None,
        );
    }

    #[test]
    fn rejects_unrecognized_shapes() {
        assert_eq!(parse_ingredient(&json!({"type": "forge:compound"})), None);
    }
}
