//! Interning item ids as recipes reference them, and remapping their provisional indices to
//! final ones once every recipe has been read.

use std::collections::{BTreeMap, BTreeSet};

use super::{RecipeDraft, with_namespace};
use crate::model::{ItemIndex, Output, Slot};

/// A plain registered id and a synthetic one (folded from a base id and an NBT string or
/// circuit configuration, see `shapes::fold_id_suffix`) are treated identically here.
///
/// None of this is final until every recipe file has been seen because that's the only point
/// at which the full set of referenced ids is known, which is needed to sort them
/// (determinism).So [`ItemInterner::intern`] hands out a provisional index for now, in
/// first-seen-order. [`ItemInterner::finish`] merges every interned id with every registered
/// one (so a registered-but-unreferenced id still gets a place in `Graph::items`), sorts the
/// result, and hands back a table remapping every provisional index to its final one.
#[derive(Debug, Default)]
pub(super) struct ItemInterner {
    /// Every id referenced so far, to its provisional index.
    by_key: BTreeMap<String, ItemIndex>,
}

impl ItemInterner {
    /// Interns `key`, returning its provisional index. Stable across repeated calls with the
    /// same key.
    ///
    /// `key` gets [`with_namespace`] first, so `dirt` and `minecraft:dirt` are one item.
    ///
    /// # Panics
    ///
    /// Only if a single export references more than [`u32::MAX`] distinct ids, which isn't a
    /// real export this tool will ever see.
    pub(super) fn intern(&mut self, raw_key: &str) -> ItemIndex {
        let key = with_namespace(raw_key);
        if let Some(&index) = self.by_key.get(key.as_ref()) {
            return index;
        }
        let index = ItemIndex::new(u32::try_from(self.by_key.len()).expect(
            "fewer than u32::MAX referenced ids in one export. \
            Please open an issue if this is a problem for you.",
        ));
        let _: Option<ItemIndex> = self.by_key.insert(key.into_owned(), index);
        index
    }

    /// Consumes every interned id, merges it with every id in `registered` (so a
    /// registered-but-unreferenced id still gets a place in the final set), and returns the
    /// merged, sorted key list plus a table mapping a provisional index to its final one.
    pub(super) fn finish(
        self,
        registered: impl Iterator<Item = String>,
    ) -> (Vec<String>, Vec<ItemIndex>) {
        let mut final_keys: BTreeSet<String> = self.by_key.keys().cloned().collect();
        final_keys.extend(registered);
        let sorted_keys: Vec<String> = final_keys.into_iter().collect();

        let final_index_of: BTreeMap<&str, ItemIndex> = sorted_keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let index_u32 = u32::try_from(index).expect(
                    "fewer than u32::MAX items in one export. \
                    Please open an issue if this is a problem for you.",
                );
                (key.as_str(), ItemIndex::new(index_u32))
            })
            .collect();

        let mut remap = vec![ItemIndex::new(0); self.by_key.len()];
        for (key, provisional) in &self.by_key {
            let final_index = *final_index_of
                .get(key.as_str())
                .expect("every referenced key is included in final_keys by construction");
            *remap
                .get_mut(provisional.index())
                .expect("provisional indices are 0..by_key.len(), the exact size of remap") =
                final_index;
        }
        (sorted_keys, remap)
    }
}

/// Rewrites every provisional [`ItemIndex`] in `drafts` to its final one, per `remap` (see
/// [`ItemInterner::finish`]).
pub(super) fn remap_item_indices(drafts: &mut [(String, RecipeDraft)], remap: &[ItemIndex]) {
    for (_, draft) in drafts.iter_mut() {
        for slot in &mut draft.inputs {
            let members: BTreeSet<ItemIndex> = slot
                .members()
                .iter()
                .copied()
                .map(|id| remap_index(id, remap))
                .collect();
            *slot = Slot::new(members, slot.tag(), slot.count(), slot.chance());
        }
        for output in &mut draft.outputs {
            *output = Output::new(
                remap_index(output.item(), remap),
                output.count(),
                output.chance(),
            );
        }
    }
}

/// Looks up `id`'s final index in `remap` (see [`ItemInterner::finish`]).
pub(super) fn remap_index(id: ItemIndex, remap: &[ItemIndex]) -> ItemIndex {
    remap
        .get(id.index())
        .copied()
        .expect("every id referenced while parsing was interned, so has an entry here")
}

/// Every id actually referenced by some recipe, directly or indirectly via a tag.
///
/// `direct_refs` and `used_tags` are provisional indices (see [`Resolver`](super::Resolver)).
pub(super) fn resolve_referenced(
    interner: &ItemInterner,
    direct_refs: BTreeSet<ItemIndex>,
    used_tags: BTreeSet<u32>,
    provisional_tag_members: &[BTreeSet<ItemIndex>],
) -> BTreeSet<String> {
    let index_to_key: BTreeMap<ItemIndex, &str> = interner
        .by_key
        .iter()
        .map(|(key, &index)| (index, key.as_str()))
        .collect();

    let mut referenced: BTreeSet<String> = direct_refs
        .into_iter()
        .map(|index| {
            (*index_to_key
                .get(&index)
                .expect("every direct_refs index was interned, so has an entry in by_key"))
            .to_owned()
        })
        .collect();
    for tag_position in used_tags {
        let position = usize::try_from(tag_position).expect("tag position fits usize");
        let members = provisional_tag_members.get(position).expect(
            "used_tags positions come from tag_index, in bounds for provisional_tag_members",
        );
        for member in members {
            let key = *index_to_key
                .get(member)
                .expect("every tag member index was interned, so has an entry in by_key");
            let _: bool = referenced.insert(key.to_owned());
        }
    }
    referenced
}

#[cfg(test)]
mod tests {
    use core::iter;

    use super::ItemInterner;
    use crate::model::ItemIndex;

    #[test]
    fn item_interner_assigns_final_indices_in_sorted_order_regardless_of_discovery_order() {
        let mut interner = ItemInterner::default();

        // Discovered in this order, but "a_first" sorts before "z_last".
        let z_last_provisional = interner.intern("z_last");
        let a_first_provisional = interner.intern("a_first");
        assert_eq!(z_last_provisional, ItemIndex::new(0), "first discovered");
        assert_eq!(a_first_provisional, ItemIndex::new(1));
        assert_eq!(
            interner.intern("z_last"),
            z_last_provisional,
            "interning again is stable"
        );

        let (sorted_keys, remap) = interner.finish(iter::empty());

        assert_eq!(
            sorted_keys,
            vec![
                "minecraft:a_first".to_owned(),
                "minecraft:z_last".to_owned()
            ]
        );
        // "z_last"'s provisional index was 0.
        assert_eq!(
            remap.first().copied(),
            Some(ItemIndex::new(1)),
            "z_last sorts second, so index 1"
        );
        // "a_first"'s provisional index was 1.
        assert_eq!(
            remap.get(1).copied(),
            Some(ItemIndex::new(0)),
            "a_first sorts first, so index 0"
        );
    }

    #[test]
    fn item_interner_treats_an_id_without_a_namespace_as_a_minecraft_id() {
        let mut interner = ItemInterner::default();

        assert_eq!(interner.intern("dirt"), interner.intern("minecraft:dirt"));
    }

    #[test]
    fn item_interner_finish_includes_a_registered_but_unreferenced_id() {
        let mut interner = ItemInterner::default();
        let _provisional = interner.intern("minecraft:referenced");

        let (sorted_keys, _remap) =
            interner.finish(iter::once("minecraft:unreferenced".to_owned()));

        assert_eq!(
            sorted_keys,
            vec![
                "minecraft:referenced".to_owned(),
                "minecraft:unreferenced".to_owned(),
            ]
        );
    }
}
