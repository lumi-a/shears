//! Turns [`Why`] and [`Cause`] into human-readable report text.
//!
//! No new analysis happens here, this just renders data that
//! [`transitively_required`](super::transitively_required) and [`RuleSet`] already computed.
//!
//! TODO: Should this perhaps be JSON or some other formatted output instead, so other tools
//! can parse this more easily and reliably?

use super::edges::Cause;
use super::transitively_required::{TransitivelyRequired, Why};
use crate::model::{Graph, ItemIndex};
use crate::rules::RuleSet;

/// Renders why a state was assigned, as one clause of report text.
///
/// `rule_sets` must be indexed the same way [`RuleSet::apply`]'s `file` argument was when the
/// rules ran.
#[inline]
#[must_use]
pub fn cause_text(rule_sets: &[RuleSet], cause: &Cause) -> String {
    match cause {
        Cause::Default => "no rule touched this".to_owned(),
        Cause::Pseudo => "built in (a pseudo-item)".to_owned(),
        Cause::Rule { file, kind, index } => rule_sets
            .get(usize::from(*file))
            .and_then(|rules| rules.reason(*kind, *index))
            .map_or_else(
                || format!("rule {kind:?}[{index}] (file {file} not loaded)"),
                |reason| format!("rule \"{reason}\""),
            ),
    }
}

/// A human-readable witness path for why `item` is transitively required.
///
/// Follows [`Why`]'s chain of BFS parents back to the required item that started it, e.g. `ore
/// ->[mine] raw_iron ->[smelt] ingot ->[press] plate`.
///
/// `None` if `item` isn't transitively required at all.
///
/// `rule_sets` must be indexed the same way [`RuleSet::apply`]'s `file` argument was when the
/// rules ran.
///
/// # Panics
///
/// If `item`, or any item/recipe reached while walking the chain, is out of bounds for
/// `graph`. Can't happen if `live` was computed from `graph`.
#[inline]
#[must_use]
pub fn witness(
    graph: &Graph,
    live: &TransitivelyRequired,
    rule_sets: &[RuleSet],
    item: ItemIndex,
) -> Option<String> {
    let mut text = graph.item(item).id().to_owned();
    let mut current = item;
    loop {
        match live.why(current)? {
            Why::Required(cause) => {
                text.push_str(" (required: ");
                text.push_str(&cause_text(rule_sets, cause));
                text.push(')');
                return Some(text);
            }
            Why::Via { recipe, output } => {
                text.push_str(" →[");
                text.push_str(graph.recipe(*recipe).id());
                text.push_str("] ");
                text.push_str(graph.item(*output).id());
                current = *output;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cause_text, witness};
    use crate::analysis::edges::{Cause, EdgeStates, NodeAttrs, RuleKind};
    use crate::analysis::transitively_required::transitively_required;
    use crate::graph_dsl::{item, parse};
    use crate::rules::RuleSet;

    #[test]
    fn witness_walks_the_full_chain_to_a_required_item() {
        let (graph, required_items) = parse(
            "
            required iron_pickaxe
            iron_pickaxe:                      3 iron_ingot + 2 stick -> iron_pickaxe
            iron_ingot_from_smelting_raw_iron: raw_iron -> iron_ingot
            raw_iron:                          raw_iron_block -> 9 raw_iron
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);
        let live = transitively_required(&graph, &attrs, &edges);

        let raw_iron_block = item(&graph, "raw_iron_block");

        let text = witness(&graph, &live, &[], raw_iron_block)
            .expect("raw_iron_block is transitively required");

        assert_eq!(
            text,
            "raw_iron_block →[raw_iron] raw_iron →[iron_ingot_from_smelting_raw_iron] \
             iron_ingot →[iron_pickaxe] iron_pickaxe (required: built in (a pseudo-item))"
        );
    }

    #[test]
    fn witness_names_the_rule_behind_a_required_item() {
        let (graph, _required) = parse(
            "
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "players mine with it"
            filter = { id = "iron_pickaxe" }
            action = { required = true }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);
        rules.apply(&graph, &mut attrs, &mut edges, 0);
        let live = transitively_required(&graph, &attrs, &edges);

        let iron_pickaxe = item(&graph, "iron_pickaxe");

        let text = witness(&graph, &live, &[rules], iron_pickaxe)
            .expect("iron_pickaxe is required directly");

        assert_eq!(
            text,
            "iron_pickaxe (required: rule \"players mine with it\")"
        );
    }

    #[test]
    fn witness_returns_none_for_a_dead_item() {
        let (graph, _required) = parse(
            "
            iron_pickaxe: 3 iron_ingot + 2 stick -> iron_pickaxe
            ",
        );
        let attrs = NodeAttrs::defaults(&graph);
        let edges = EdgeStates::defaults(&graph);
        let live = transitively_required(&graph, &attrs, &edges);

        assert!(witness(&graph, &live, &[], item(&graph, "iron_pickaxe")).is_none());
    }

    #[test]
    fn cause_text_describes_default_and_pseudo() {
        assert_eq!(cause_text(&[], &Cause::Default), "no rule touched this");
        assert_eq!(cause_text(&[], &Cause::Pseudo), "built in (a pseudo-item)");
    }

    #[test]
    fn cause_text_falls_back_gracefully_for_an_unresolvable_rule() {
        let cause = Cause::Rule {
            file: 0,
            kind: RuleKind::Item,
            index: 0,
        };

        let text = cause_text(&[], &cause);

        assert!(
            text.contains("not loaded"),
            "no rule_sets were passed, so file 0 can't resolve: {text}"
        );
    }
}
