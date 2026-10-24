//! A tiny path language for pulling values out of a recipe's dumped JSON.
//!
//! - `content.ingredient.item` walks object keys
//! - `content.value[0].fluid` adds an array index
//! - `""` (the empty path) means the node itself

use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// One step of a [`Path`], either an object key or an array index.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// A key into a JSON object.
    Key(String),
    /// An index into a JSON array.
    Index(usize),
}

/// A parsed path, ready to walk a [`Value`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Path(Vec<Segment>);

/// A path string wasn't well-formed.
#[derive(Debug, thiserror::Error)]
#[error("invalid path {raw:?}: {reason}")]
pub(super) struct PathError {
    /// The path string that failed to parse.
    raw: String,
    /// What was wrong with it.
    reason: &'static str,
}

impl Path {
    /// Parses a dotted path like `content.value[0].fluid`.
    ///
    /// The empty string is a valid path, resolving to the node it's given.
    ///
    /// # Errors
    ///
    /// If a segment's bracketed suffix isn't a valid array index, or a segment is empty
    /// where a key was expected (e.g. `foo..bar`, `foo[0].`).
    pub(super) fn parse(raw: &str) -> Result<Self, PathError> {
        if raw.is_empty() {
            return Ok(Self(Vec::new()));
        }

        let mut segments = Vec::new();
        for piece in raw.split('.') {
            let (key_part, index_part) = match piece.strip_suffix(']') {
                Some(rest) => {
                    let Some((key_part, index_str)) = rest.split_once('[') else {
                        return Err(PathError {
                            raw: raw.to_owned(),
                            reason: "unmatched ']'",
                        });
                    };
                    let index = index_str.parse::<usize>().map_err(|_err| PathError {
                        raw: raw.to_owned(),
                        reason: "the bracketed part isn't a non-negative integer",
                    })?;
                    (key_part, Some(index))
                }
                None if piece.contains('[') => {
                    return Err(PathError {
                        raw: raw.to_owned(),
                        reason: "unmatched '['",
                    });
                }
                None => (piece, None),
            };

            if key_part.is_empty() {
                return Err(PathError {
                    raw: raw.to_owned(),
                    reason: "empty key segment",
                });
            }
            segments.push(Segment::Key(key_part.to_owned()));
            if let Some(index) = index_part {
                segments.push(Segment::Index(index));
            }
        }
        Ok(Self(segments))
    }

    /// Walks `value` by this path. Returns final node if every step resolves.
    pub(super) fn resolve<'v>(&self, value: &'v Value) -> Option<&'v Value> {
        let mut current = value;
        for segment in &self.0 {
            current = match segment {
                Segment::Key(key) => current.as_object()?.get(key)?,
                Segment::Index(index) => current.as_array()?.get(*index)?,
            };
        }
        Some(current)
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Path;

    #[test]
    fn empty_path_resolves_to_the_node_itself() {
        let value = json!("minecraft:iron_ingot");
        let path = Path::parse("").expect("an empty path is valid");
        assert_eq!(path.resolve(&value), Some(&value));
    }

    #[test]
    fn walks_nested_object_keys() {
        let value = json!({"content": {"ingredient": {"item": "gtceu:wood_axe"}}});
        let path = Path::parse("content.ingredient.item").expect("a valid path");
        assert_eq!(path.resolve(&value), Some(&json!("gtceu:wood_axe")));
    }

    #[test]
    fn walks_an_array_index() {
        let value = json!({"content": {"value": [{"fluid": "gtceu:oxygen"}]}});
        let path = Path::parse("content.value[0].fluid").expect("a valid path");
        assert_eq!(path.resolve(&value), Some(&json!("gtceu:oxygen")));
    }

    #[test]
    fn missing_key_resolves_to_none_rather_than_erroring() {
        let value = json!({"content": {"item": "gtceu:wood_axe"}});
        let path = Path::parse("content.ingredient.item").expect("a valid path");
        assert_eq!(path.resolve(&value), None);
    }

    #[test]
    fn out_of_bounds_index_resolves_to_none() {
        let value = json!({"value": [{"fluid": "gtceu:oxygen"}]});
        let path = Path::parse("value[5]").expect("a valid path");
        assert_eq!(path.resolve(&value), None);
    }

    #[test]
    fn rejects_an_unmatched_closing_bracket() {
        drop(Path::parse("value[0").expect_err("missing '['"));
    }

    #[test]
    fn rejects_a_non_numeric_index() {
        drop(Path::parse("value[abc]").expect_err("not a valid index"));
    }

    #[test]
    fn rejects_an_empty_key_segment() {
        drop(Path::parse("content..item").expect_err("empty segment between the dots"));
    }
}
