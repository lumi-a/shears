//! Searching the graph with selectors.

use shears_core::rules::selector::{self, Context, Selector};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use itertools::Itertools as _;
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

/// A plain-text query. These are matched fuzzily.
#[derive(Debug, PartialEq, Eq)]
struct TextQuery {
    /// The query.
    text: String,
}

impl TextQuery {
    /// `query` as plain text. `None` if it's empty, TOML, or a `/regex/`.
    fn parse(query: &str) -> Option<Self> {
        let trimmed = query.trim();
        if trimmed.is_empty() || trimmed.starts_with(['{', '[', '"', '/']) {
            return None;
        }
        Some(Self {
            text: trimmed.to_owned(),
        })
    }

    /// The positions in `ids` that match with the relevant score. Unsorted.
    ///
    /// The whole id is matched, so a word can also pick a mod, e.g. `gtceu iron dust`.
    ///
    /// Thus, a mod whose name contains the word can crowd out the rest, e.g. `ironjetpacks:*`
    /// for `iron`. In this case, use frizbee's `!` to exclude: `iron !^iron`.
    fn matches(&self, ids: &[&str]) -> Vec<(u16, usize)> {
        frizbee::Matcher::from_query(&self.text, &frizbee::Config::default())
            .match_list(ids)
            .into_iter()
            .filter_map(|found| Some((found.score, usize::try_from(found.index).ok()?)))
            .collect()
    }
}

/// One match of a plain-text search.
struct Hit<'graph> {
    /// frizbee's score. Higher is better.
    score: u16,
    /// The matched id.
    id: &'graph str,
    /// Whether it's an item or a recipe.
    kind: Kind,
    /// See [`Link::live`].
    live: Option<bool>,
}

#[wasm_bindgen]
#[expect(
    clippy::multiple_inherent_impl,
    reason = "the explorer's part of Session's JS API. Kept apart from lib.rs's core lifecycle"
)]
impl Session {
    /// Finds the items and recipes matching `query`, as JSON mirrored by `SearchResults` in
    /// `web/src/session-protocol.ts`. Lists only the kinds asked for, at most `limit` of
    /// them. A selector's are sorted by id, items and recipes mixed.
    ///
    /// A query is one of the following:
    /// - `/regex/` for ids
    /// - a selector as in a rule's `filter`. A selector is read as an item selector and as a
    ///   recipe selector. If it's not a valid selector for recipes (respectively items), it
    ///   doesn't match recipes (respectively items).
    /// - or plain text. This is matched fuzzily.
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
        if let Some(text) = TextQuery::parse(query) {
            return serde_json::to_string(&search_text(self, &text, items, recipes, limit))
                .map_err(|error| js_error(&error));
        }
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
        let item_link = |item: ItemIndex| Link {
            kind: Kind::Item,
            id: graph.item(item).id().to_owned(),
            live: liveness.map(|live| live.is_transitively_required(item)),
        };
        let recipe_link = |recipe: RecipeIndex| Link {
            kind: Kind::Recipe,
            id: graph.recipe(recipe).id().to_owned(),
            live: liveness.map(|live| live.is_recipe_transitively_required(recipe)),
        };
        // Both are in index order, so sorted by id already. Merging them keeps it that way,
        // with an item before a recipe of the same id.
        // This is done lazily, so only the links shown are made.
        let merged = shown_items.iter().copied().map(item_link).merge_by(
            shown_recipes.iter().copied().map(recipe_link),
            |item, recipe| item.id <= recipe.id,
        );
        let result = SearchResults {
            item_count: item_matches.as_ref().map(Vec::len),
            recipe_count: recipe_matches.as_ref().map(Vec::len),
            links: Links {
                total: shown_items.len() + shown_recipes.len(),
                shown: merged.take(limit).collect(),
            },
        };
        serde_json::to_string(&result).map_err(|error| js_error(&error))
    }
}

/// [`Session::search_json`] for plain text.
fn search_text(
    session: &Session,
    query: &TextQuery,
    items: bool,
    recipes: bool,
    limit: usize,
) -> SearchResults {
    let graph = &session.graph;
    let liveness = session.analyzed.as_ref().map(|analyzed| &analyzed.live);

    let item_indices: Vec<ItemIndex> = graph.item_indices().collect();
    let item_ids: Vec<&str> = item_indices
        .iter()
        .map(|&item| graph.item(item).id())
        .collect();
    let item_matches = query.matches(&item_ids);
    let recipe_indices: Vec<RecipeIndex> = graph.recipe_indices().collect();
    let recipe_ids: Vec<&str> = recipe_indices
        .iter()
        .map(|&recipe| graph.recipe(recipe).id())
        .collect();
    let recipe_matches = query.matches(&recipe_ids);

    let mut hits = Vec::new();
    if items {
        hits.extend(item_matches.iter().filter_map(|&(score, position)| {
            let item = *item_indices.get(position)?;
            let id = *item_ids.get(position)?;
            Some(Hit {
                score,
                id,
                kind: Kind::Item,
                live: liveness.map(|live| live.is_transitively_required(item)),
            })
        }));
    }
    if recipes {
        hits.extend(recipe_matches.iter().filter_map(|&(score, position)| {
            let recipe = *recipe_indices.get(position)?;
            let id = *recipe_ids.get(position)?;
            Some(Hit {
                score,
                id,
                kind: Kind::Recipe,
                live: liveness.map(|live| live.is_recipe_transitively_required(recipe)),
            })
        }));
    }
    // Best first. Among equals the shorter id, as it has less besides the match, then by
    // id, then items before recipes (the sort is stable).
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.id.len().cmp(&b.id.len()))
            .then(a.id.cmp(b.id))
    });

    SearchResults {
        item_count: Some(item_matches.len()),
        recipe_count: Some(recipe_matches.len()),
        links: Links {
            total: hits.len(),
            shown: hits
                .into_iter()
                .take(limit)
                .map(|hit| Link {
                    kind: hit.kind,
                    id: hit.id.to_owned(),
                    live: hit.live,
                })
                .collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use shears_core::rules::selector::Context;

    use super::{TextQuery, parse_query};

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

    #[test]
    fn plain_text_is_anything_but_toml_or_a_regex() {
        assert_eq!(
            TextQuery::parse(" iron dust "),
            Some(TextQuery {
                text: "iron dust".to_owned(),
            })
        );
        assert_eq!(TextQuery::parse("/iron/"), None);
        assert_eq!(TextQuery::parse("{ mod = \"gtceu\" }"), None);
        assert_eq!(TextQuery::parse("  "), None);
    }

    #[test]
    fn text_matches_whole_ids_and_frizbee_syntax() {
        let ids = ["minecraft:iron_ingot", "ironfurnaces:copper_furnace"];
        let positions = |query: &str| {
            let mut found: Vec<usize> = TextQuery::parse(query)
                .expect("plain text")
                .matches(&ids)
                .into_iter()
                .map(|(_, position)| position)
                .collect();
            found.sort_unstable();
            found
        };

        assert_eq!(positions("iron"), [0, 1]);
        assert_eq!(positions("iron !^iron"), [0]);
        assert_eq!(positions("ironfurnaces:"), [1]);
    }
}
