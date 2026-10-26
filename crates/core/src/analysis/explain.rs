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
    use crate::graph_dsl::parse;
    use crate::model::ItemIndex;
    use crate::rules::RuleSet;

    #[test]
    fn witness_walks_the_full_chain_to_a_required_item() {
        let (graph, required_items) = parse(
            "
            required plate
            press: ingot -> plate
            smelt: raw -> ingot
            mine:  ore -> raw
            ",
        );
        let mut attrs = NodeAttrs::defaults(&graph);
        for required in required_items {
            attrs.set_required(required, Cause::Pseudo);
        }
        let edges = EdgeStates::defaults(&graph);
        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: ingot=0, ore=1, plate=2, raw=3.
        let ore = ItemIndex::new(1);

        let text = witness(&graph, &live, &[], ore).expect("ore is transitively required");

        assert_eq!(
            text,
            "ore →[mine] raw →[smelt] ingot →[press] plate \
             (required: built in (a pseudo-item))"
        );
    }

    #[test]
    fn witness_names_the_rule_behind_a_required_item() {
        let (graph, _required) = parse(
            "
            mix: stone_dust + water -> concrete
            ",
        );
        let rules = RuleSet::parse(
            r#"
            [[item-rule]]
            reason = "concrete is required for building"
            filter = { id = "concrete" }
            action = { required = true }
            "#,
        )
        .expect("valid rules");
        let mut attrs = NodeAttrs::defaults(&graph);
        let mut edges = EdgeStates::defaults(&graph);
        rules.apply(&graph, &mut attrs, &mut edges, 0);
        let live = transitively_required(&graph, &attrs, &edges);

        // Sorted item order: concrete=0, stone_dust=1, water=2.
        let concrete = ItemIndex::new(0);

        let text =
            witness(&graph, &live, &[rules], concrete).expect("concrete is required directly");

        assert_eq!(
            text,
            "concrete (required: rule \"concrete is required for building\")"
        );
    }

    #[test]
    fn witness_returns_none_for_a_dead_item() {
        let (graph, _required) = parse(
            "
            mix: stone_dust + water -> concrete
            ",
        );
        let attrs = NodeAttrs::defaults(&graph);
        let edges = EdgeStates::defaults(&graph);
        let live = transitively_required(&graph, &attrs, &edges);

        assert!(witness(&graph, &live, &[], ItemIndex::new(0)).is_none());
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
