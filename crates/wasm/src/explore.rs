//! Searching the graph with selectors.

use shears_core::rules::selector::{self, Context, Selector};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use serde::Serialize;
use shears_core::model::{ItemIndex, RecipeIndex};

use crate::{Kind, Link, Links, Session, js_error};

/// What a search found.
#[derive(Debug, Serialize)]
struct SearchResults {
    /// How many items match. `None` if the query isn't an item selector.
    item_count: Option<usize>,
    /// How many recipes match. `None` if the query isn't a recipe selector.
    recipe_count: Option<usize>,
    /// The matches of the kinds asked for.
    links: Links,
}

/// What a TOML query is prefixed with to parse it as a document.
const QUERY_PREFIX: &str = "filter = ";

/// Turns a search query into a selector, `None` for an empty selector (which matches
/// everything).
///
/// A query starting with `{`, `[` or `"` is TOML, the same as a rule's `filter`.
/// Anything else is one id, or a `/regex/`, as if it were quoted.
///
/// # Errors
///
/// If the query is TOML but doesn't parse, or isn't a valid selector in `context`.
fn parse_query(query: &str, context: Context) -> Result<Option<Selector>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value = if trimmed.starts_with(['{', '[', '"']) {
        let mut document: toml::Table = toml::from_str(&format!("{QUERY_PREFIX}{trimmed}"))
            .map_err(|error| {
                // The full message would quote QUERY_PREFIX back, and count columns with it.
                // toml leaves the message empty when the query ends too early, for some
                // reason?
                let message = match error.message() {
                    "" => "incomplete query",
                    message => message,
                };
                error.span().map_or_else(
                    || message.to_owned(),
                    |span| {
                        let column = span.start.saturating_sub(QUERY_PREFIX.len()) + 1;
                        format!("{message} (column {column})")
                    },
                )
            })?;
        document
            .remove("filter")
            .ok_or_else(|| "expected a single selector".to_owned())?
    } else {
        toml::Value::String(trimmed.to_owned())
    };
    selector::parse(&value, context)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[wasm_bindgen]
#[expect(
    clippy::multiple_inherent_impl,
    reason = "the explorer's part of Session's JS API. Kept apart from lib.rs's core lifecycle"
)]
impl Session {
    /// Finds the items and recipes matching `query`, as JSON mirrored by `SearchResults` in
    /// `web/src/session-protocol.ts`. Lists only the kinds asked for, at most `limit` of
    /// them. Items first.
    ///
    /// A query is a selector as in a rule's `filter`, or plain text as one id or `/regex/`.
    /// It's read as an item selector and as a recipe selector. A kind it's no valid selector
    /// for (e.g. recipes, for `tag`) has no matches.
    ///
    /// # Errors
    ///
    /// If `query` isn't a valid selector for either kind.
    pub fn search_json(
        &self,
        query: &str,
        items: bool,
        recipes: bool,
        limit: usize,
    ) -> Result<String, JsValue> {
        let item_filter = parse_query(query, Context::Item);
        let recipe_filter = parse_query(query, Context::Recipe);
        if let (Err(item_error), Err(recipe_error)) = (&item_filter, &recipe_filter) {
            let message = if item_error == recipe_error {
                item_error.clone()
            } else {
                format!("as items: {item_error}. As recipes: {recipe_error}")
            };
            return Err(js_error(&message));
        }
        let liveness = self.analyzed.as_ref().map(|analyzed| &analyzed.live);
        let graph = &self.graph;

        let item_matches: Option<Vec<ItemIndex>> = item_filter.ok().map(|filter| {
            graph
                .item_indices()
                .filter(|&item| {
                    filter
                        .as_ref()
                        .is_none_or(|matching| selector::matches_item(matching, graph, item))
                })
                .collect()
        });
        let recipe_matches: Option<Vec<RecipeIndex>> = recipe_filter.ok().map(|filter| {
            graph
                .recipe_indices()
                .filter(|&recipe| {
                    filter.as_ref().is_none_or(|matching| {
                        selector::matches_recipe(matching, graph, recipe)
                    })
                })
                .collect()
        });

        let shown_items = item_matches
            .as_deref()
            .filter(|_| items)
            .unwrap_or_default();
        let shown_recipes = recipe_matches
            .as_deref()
            .filter(|_| recipes)
            .unwrap_or_default();
        let item_links = shown_items.iter().map(|&item| Link {
            kind: Kind::Item,
            id: graph.item(item).id().to_owned(),
            live: liveness.map(|live| live.is_transitively_required(item)),
        });
        let recipe_links = shown_recipes.iter().map(|&recipe| Link {
            kind: Kind::Recipe,
            id: graph.recipe(recipe).id().to_owned(),
            live: liveness.map(|live| live.is_recipe_transitively_required(recipe)),
        });
        let result = SearchResults {
            item_count: item_matches.as_ref().map(Vec::len),
            recipe_count: recipe_matches.as_ref().map(Vec::len),
            links: Links {
                total: shown_items.len() + shown_recipes.len(),
                shown: item_links.chain(recipe_links).take(limit).collect(),
            },
        };
        serde_json::to_string(&result).map_err(|error| js_error(&error))
    }
}

#[cfg(test)]
mod tests {
    use shears_core::rules::selector::Context;

    use super::parse_query;

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches!(parse_query("  ", Context::Item), Ok(None)));
    }

    #[test]
    fn plain_text_is_an_id_or_a_regex() {
        assert!(matches!(
            parse_query("minecraft:dirt", Context::Item),
            Ok(Some(_))
        ));
        assert!(matches!(parse_query("/dirt$/", Context::Item), Ok(Some(_))));
    }

    #[test]
    fn a_table_is_a_selector_checked_against_its_context() {
        assert!(matches!(
            parse_query(r#"{ tag = "forge:ingots" }"#, Context::Item),
            Ok(Some(_))
        ));
        let error = parse_query(r#"{ tag = "forge:ingots" }"#, Context::Recipe)
            .expect_err("tag doesn't apply to recipes");
        assert!(error.contains("tag"), "{error}");
    }

    #[test]
    fn toml_errors_count_columns_from_the_query_itself() {
        let error = parse_query("{ mod = 1 2 }", Context::Item).expect_err("invalid TOML");
        assert!(error.ends_with("(column 11)"), "{error}");
    }

    #[test]
    fn a_query_that_ends_too_early_says_so() {
        let error = parse_query("{ id =", Context::Item).expect_err("incomplete TOML");
        assert_eq!(error, "incomplete query (column 7)");
    }
}
