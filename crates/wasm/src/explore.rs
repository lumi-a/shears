//! Searching the graph with selectors.

use itertools::Itertools as _;
use serde::Serialize;
use shears_core::analysis::transitively_required::TransitivelyRequired;
use shears_core::model::{Graph, ItemIndex, RecipeIndex};
use shears_core::rules::selector::{self, Context, Selector};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{Kind, Link, Links, Node, Session, js_error};

/// What a search found.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
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
/// A query starting with `{` or `[` is TOML, the same as a rule's `filter`.
/// Anything else is one id, or a `/regex/`, as if it were quoted.
///
/// # Errors
///
/// If the query is TOML but doesn't parse, or isn't a valid selector in `context`.Columns
/// count from `start` on, where `query` starts in the whole query.
fn parse_query(
    query: &str,
    start: usize,
    context: Context,
) -> Result<Option<Selector>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value = if trimmed.starts_with(['{', '[']) {
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
                        let column = span.start.saturating_sub(QUERY_PREFIX.len()) + start + 1;
                        format!("{message} (column {column})")
                    },
                )
            })?;
        document
            .remove("filter")
            .ok_or_else(|| "expected a single selector".to_owned())?
    } else if trimmed.starts_with('/') && (trimmed.len() < 2 || !trimmed.ends_with('/')) {
        return Err(format!("unclosed /regex/ (column {})", start + 1));
    } else {
        toml::Value::String(trimmed.to_owned())
    };
    selector::parse(&value, context)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Parses each of `selectors` (see [`Query::selectors`]) in `context`. `None` if there are no
/// selectors.
///
/// In a query, ALL selectors have to match.
///
/// # Errors
///
/// The first selector's that isn't valid, see [`parse_query`].
fn parse_selectors(
    selectors: &[(usize, &str)],
    context: Context,
) -> Result<Option<Selector>, String> {
    let mut parsed: Vec<Selector> = selectors
        .iter()
        .filter_map(|&(start, selector)| parse_query(selector, start, context).transpose())
        .collect::<Result<_, _>>()?;
    Ok(if parsed.len() > 1 {
        Some(Selector::All(parsed))
    } else {
        parsed.pop()
    })
}

/// A search query, split into selectors and plain text.
#[derive(Debug, PartialEq, Eq)]
struct Query<'query> {
    /// The selectors, for [`parse_query`], each with where it starts in the trimmed query.
    ///
    /// In a query, ALL selectors have to match.
    selectors: Vec<(usize, &'query str)>,
    /// Everything else, `None` if that's empty.
    text: Option<TextQuery>,
}

impl<'query> Query<'query> {
    /// Splits `query` into its selectors and its text, e.g. `iron { mod = "gtceu" } /rod/`
    /// into `{ mod = "gtceu" }`, `/rod/` and `iron`.
    ///
    /// A selector starts a word, and is
    /// - a `{...}` or `[...]`, up to its matching bracket, or to the end if there's none,
    /// - or a `/regex/`, up to the next `/` that ends a word, or to the end if there's none.
    ///
    /// Everything else is text, including words with a `{`, `[` or `/` further in, e.g.
    /// `circuit{configuration:1}` or `macerator/iron`.
    ///
    /// # Errors
    ///
    /// If a selector doesn't end a word either, e.g. in `{ mod = "gtceu" }iron`.
    fn split(query: &'query str) -> Result<Self, String> {
        let trimmed = query.trim();
        let mut selectors = Vec::new();
        let mut words = Vec::new();
        let mut rest = trimmed;
        loop {
            rest = rest.trim_start();
            let start = trimmed.len() - rest.len();
            let length = if rest.is_empty() {
                break;
            } else if rest.starts_with(['{', '[']) {
                bracketed_length(rest)
            } else if rest.starts_with('/') {
                regex_length(rest)
            } else {
                let word_length = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let (word, after) = rest.split_at(word_length);
                words.push(word);
                rest = after;
                continue;
            };
            let (selector, after) = rest.split_at(length);
            if after
                .chars()
                .next()
                .is_some_and(|next| !next.is_whitespace())
            {
                return Err(format!(
                    "expected a space after the selector (column {})",
                    start + length + 1
                ));
            }
            selectors.push((start, selector));
            rest = after;
        }
        Ok(Self {
            selectors,
            text: TextQuery::new(&words.join(" ")),
        })
    }
}

/// How long the `/regex/` that `regex` starts with is, i.e. up to and including the next `/`
/// that's followed by whitespace or the end. All of `regex` if there's none.
fn regex_length(regex: &str) -> usize {
    regex
        .char_indices()
        .skip(1)
        .find(|&(position, character)| {
            character == '/'
                && regex
                    .get(position + 1..)
                    .and_then(|after| after.chars().next())
                    .is_none_or(char::is_whitespace)
        })
        .map_or(regex.len(), |(position, _)| position + 1)
}

/// How long the bracketed TOML value `toml` starts with is, i.e. up to and including the
/// bracket that closes its first one. All of `toml` if that's never closed.
///
/// Brackets inside strings don't count.
fn bracketed_length(toml: &str) -> usize {
    let mut depth = 0_usize;
    // The quote of the string we're in, if any.
    let mut quote = None;
    let mut escaped = false;
    for (position, character) in toml.char_indices() {
        match (quote, character) {
            (Some('"'), _) if escaped => escaped = false,
            (Some('"'), '\\') => escaped = true,
            (Some(open), _) if character == open => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '{' | '[') => depth += 1,
            (None, '}' | ']') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return position + 1;
                }
            }
            _ => {}
        }
    }
    toml.len()
}

/// Plain text, matched fuzzily.
#[derive(Debug, PartialEq, Eq)]
struct TextQuery {
    /// The text.
    text: String,
}

impl TextQuery {
    /// `text`, trimmed. `None` if that's empty.
    fn new(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| Self {
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
    /// them, items and recipes mixed.
    ///
    /// A query is selectors and plain text, either of them optional, e.g.
    /// `iron { mod = "gtceu" } /rod$/`:
    /// - A selector is a `/regex/` for ids, or one as in a rule's `filter`. All of them have
    ///   to match. They're read as item selectors and as recipe selectors. If they're not
    ///   valid for recipes (respectively items), they don't match recipes (respectively
    ///   items).
    /// - The text is matched fuzzily against the ids of what the selectors match, or of
    ///   everything without one. Results are then sorted best first, and otherwise by id.
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
        let parts = Query::split(query).map_err(|error| js_error(&error))?;
        let item_filter = parse_selectors(&parts.selectors, ItemIndex::CONTEXT);
        let recipe_filter = parse_selectors(&parts.selectors, RecipeIndex::CONTEXT);
        if let (Err(item_error), Err(recipe_error)) = (&item_filter, &recipe_filter) {
            let message = if item_error == recipe_error {
                item_error.clone()
            } else {
                format!("as items: {item_error}. As recipes: {recipe_error}")
            };
            return Err(js_error(&message));
        }
        let graph = &self.graph;
        let liveness = self.analyzed.as_ref().map(|analyzed| &analyzed.live);

        let item_matches: Option<Vec<ItemIndex>> = item_filter
            .ok()
            .map(|filter| matching(graph, filter.as_ref()));
        let recipe_matches: Option<Vec<RecipeIndex>> = recipe_filter
            .ok()
            .map(|filter| matching(graph, filter.as_ref()));

        if let Some(text) = &parts.text {
            let item_hits = item_matches
                .as_deref()
                .map(|nodes| text_hits(graph, liveness, text, nodes));
            let recipe_hits = recipe_matches
                .as_deref()
                .map(|nodes| text_hits(graph, liveness, text, nodes));
            let result = ranked(item_hits, recipe_hits, items, recipes, limit);
            return serde_json::to_string(&result).map_err(|error| js_error(&error));
        }

        let shown_items = item_matches
            .as_deref()
            .filter(|_| items)
            .unwrap_or_default();
        let shown_recipes = recipe_matches
            .as_deref()
            .filter(|_| recipes)
            .unwrap_or_default();
        // Both are in index order, so sorted by id already. Merging them keeps it that way,
        // with an item before a recipe of the same id.
        // This is done lazily, so only the links shown are made.
        let merged = shown_items
            .iter()
            .map(|&item| Link::to(item, graph, liveness))
            .merge_by(
                shown_recipes
                    .iter()
                    .map(|&recipe| Link::to(recipe, graph, liveness)),
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

/// The `N`s matching `filter`, every one for `None`, in index order.
fn matching<N: Node>(graph: &Graph, filter: Option<&Selector>) -> Vec<N> {
    N::all(graph)
        .filter(|&node| filter.is_none_or(|selector| node.matches(selector, graph)))
        .collect()
}

/// The `nodes` whose ids match `query`, unsorted.
fn text_hits<'graph, N: Node>(
    graph: &'graph Graph,
    liveness: Option<&TransitivelyRequired>,
    query: &TextQuery,
    nodes: &[N],
) -> Vec<Hit<'graph>> {
    let ids: Vec<&str> = nodes.iter().map(|&node| node.id(graph)).collect();
    query
        .matches(&ids)
        .into_iter()
        .filter_map(|(score, position)| {
            let node = *nodes.get(position)?;
            Some(Hit {
                score,
                id: ids.get(position)?,
                kind: N::KIND,
                live: liveness.map(|live| node.is_live(live)),
            })
        })
        .collect()
}

/// [`Session::search_json`]'s results for a query with text, from the hits for each kind.
/// `None` for a kind the selector isn't valid for.
fn ranked(
    item_hits: Option<Vec<Hit<'_>>>,
    recipe_hits: Option<Vec<Hit<'_>>>,
    items: bool,
    recipes: bool,
    limit: usize,
) -> SearchResults {
    let item_count = item_hits.as_ref().map(Vec::len);
    let recipe_count = recipe_hits.as_ref().map(Vec::len);

    let mut hits = Vec::new();
    if items {
        hits.extend(item_hits.into_iter().flatten());
    }
    if recipes {
        hits.extend(recipe_hits.into_iter().flatten());
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
        item_count,
        recipe_count,
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

    use shears_core::rules::selector::Selector;

    use super::{Query, TextQuery, parse_query, parse_selectors};

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches!(parse_query("  ", 0, Context::Item), Ok(None)));
    }

    #[test]
    fn plain_text_is_an_id_or_a_regex() {
        assert!(matches!(
            parse_query("minecraft:dirt", 0, Context::Item),
            Ok(Some(_))
        ));
        assert!(matches!(
            parse_query("/dirt$/", 0, Context::Item),
            Ok(Some(_))
        ));
    }

    #[test]
    fn a_table_is_a_selector_checked_against_its_context() {
        assert!(matches!(
            parse_query(r#"{ tag = "forge:ingots" }"#, 0, Context::Item),
            Ok(Some(_))
        ));
        let error = parse_query(r#"{ tag = "forge:ingots" }"#, 0, Context::Recipe)
            .expect_err("tag doesn't apply to recipes");
        assert!(error.contains("tag"), "{error}");
    }

    #[test]
    fn toml_errors_count_columns_from_the_query_itself() {
        let error = parse_query("{ mod = 1 2 }", 0, Context::Item).expect_err("invalid TOML");
        assert!(error.ends_with("(column 11)"), "{error}");
    }

    #[test]
    fn a_query_that_ends_too_early_says_so() {
        let error = parse_query("{ id =", 0, Context::Item).expect_err("incomplete TOML");
        assert_eq!(error, "incomplete query (column 7)");
    }

    /// [`Query::split`]'s selectors without their starts, and its text.
    fn split(query: &str) -> (Vec<&str>, Option<String>) {
        let parts = Query::split(query).expect("a valid query");
        (
            parts
                .selectors
                .into_iter()
                .map(|(_, selector)| selector)
                .collect(),
            parts.text.map(|found| found.text),
        )
    }

    #[test]
    fn a_query_without_brackets_or_regexes_is_all_text() {
        assert_eq!(split(" iron dust "), (vec![], Some("iron dust".to_owned())));
        assert_eq!(
            split(r#""minecraft:dirt" macerator/iron"#),
            (
                vec![],
                Some(r#""minecraft:dirt" macerator/iron"#.to_owned())
            )
        );
        assert_eq!(split("  "), (vec![], None));
    }

    #[test]
    fn brackets_and_slashes_inside_a_word_are_text() {
        assert_eq!(
            split("minecraft/iron sword/"),
            (vec![], Some("minecraft/iron sword/".to_owned()))
        );
        assert_eq!(
            split("circuit{configuration:1}"),
            (vec![], Some("circuit{configuration:1}".to_owned()))
        );
    }

    #[test]
    fn a_selector_has_to_end_a_word() {
        assert_eq!(
            Query::split(r#"iron {mod="gtceu"}{tag="forge:rods"}"#),
            Err("expected a space after the selector (column 19)".to_owned())
        );
        assert_eq!(
            Query::split("{ id = \"a\" }iron"),
            Err("expected a space after the selector (column 13)".to_owned())
        );
    }

    #[test]
    fn every_regex_and_table_is_a_selector() {
        assert_eq!(
            split("/iron/ rod /greg/"),
            (vec!["/iron/", "/greg/"], Some("rod".to_owned()))
        );
        assert_eq!(
            split(r#"iron {mod="gtceu"} {tag="forge:rods"}"#),
            (
                vec![r#"{mod="gtceu"}"#, r#"{tag="forge:rods"}"#],
                Some("iron".to_owned())
            )
        );
    }

    #[test]
    fn a_regex_ends_at_a_slash_ending_a_word() {
        assert_eq!(
            split("/iron/(ingot|rod)$/ dust"),
            (vec!["/iron/(ingot|rod)$/"], Some("dust".to_owned()))
        );
        assert_eq!(split("/iron [a-z]+/"), (vec!["/iron [a-z]+/"], None));
        assert_eq!(split("/iron"), (vec!["/iron"], None));
    }

    #[test]
    fn text_around_a_table_is_text() {
        assert_eq!(
            split(r#"iron { mod = "gtceu" } dust"#),
            (vec![r#"{ mod = "gtceu" }"#], Some("iron dust".to_owned()))
        );
        assert_eq!(
            split(r#"[{ id = "a" }, { id = "b" }] iron"#),
            (
                vec![r#"[{ id = "a" }, { id = "b" }]"#],
                Some("iron".to_owned())
            )
        );
    }

    #[test]
    fn brackets_in_strings_dont_end_a_table() {
        assert_eq!(
            split(r#"{ id = "/a}b/", mod = 'c}\' } iron"#),
            (
                vec![r#"{ id = "/a}b/", mod = 'c}\' }"#],
                Some("iron".to_owned())
            )
        );
        assert_eq!(
            split(r#"{ id = "a\"}" } iron"#),
            (vec![r#"{ id = "a\"}" }"#], Some("iron".to_owned()))
        );
    }

    #[test]
    fn an_unclosed_table_runs_to_the_end() {
        assert_eq!(
            split("iron { mod ="),
            (vec!["{ mod ="], Some("iron".to_owned()))
        );
    }

    #[test]
    fn error_columns_count_from_the_start_of_the_whole_query() {
        let error = |query: &str| {
            parse_selectors(
                &Query::split(query).expect("split").selectors,
                Context::Item,
            )
            .expect_err("an invalid selector")
        };
        assert_eq!(error("iron { mod ="), "incomplete query (column 13)");
        assert_eq!(error("/iron/ rod /greg"), "unclosed /regex/ (column 12)");
    }

    #[test]
    fn all_selectors_have_to_match() {
        assert!(matches!(
            parse_selectors(&Query::split("/a/ /b/").expect("split").selectors, Context::Item),
            Ok(Some(Selector::All(all))) if all.len() == 2
        ));
        assert!(matches!(
            parse_selectors(
                &Query::split("/a/").expect("split").selectors,
                Context::Item
            ),
            Ok(Some(Selector::Id(_)))
        ));
        assert!(matches!(
            parse_selectors(&Query::split("a").expect("split").selectors, Context::Item),
            Ok(None)
        ));
    }

    #[test]
    fn text_matches_whole_ids_and_frizbee_syntax() {
        let ids = ["minecraft:iron_ingot", "ironfurnaces:copper_furnace"];
        let positions = |query: &str| {
            let mut found: Vec<usize> = TextQuery::new(query)
                .expect("some text")
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
