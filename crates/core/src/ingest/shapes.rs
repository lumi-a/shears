//! Declarative recipe shapes via a config file.
//!
//! This means the user can declare their own recipe shapes instead of relying on whatever
//! hardcoded ones ship with the program.
//!
//! TODO: Not sure if shapes.toml is the right place for this, but it would be nice if pack
//! authors could also specify recipes to be edited. E.g. in Monifactory's omnic synthesis, the
//! mechanics for how you gain omnium is not at all well-described by the recipe files. The mod
//! authors may then want to remove those recipes, and instead supply one of their own (in the
//! case of omnium: "take all possible elements and turn them into X omnium nuggets").

use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::OnceLock;

#[cfg(test)]
use crate::ingest::DEFAULT_SHAPES_TOML;

use serde::Deserialize;
use serde_json::Value;
use toml::de::Error as TomlError;

use super::ingredient::Leaf;
use super::path::Path;
use super::{resolve_leaves, resolve_output_id, slot_tag, RecipeDraft, Resolver};
use crate::model::{Output, Slot};

/// Where a [`MemberCase::Item`]'s base id comes from.
#[derive(Debug)]
enum BaseRef {
    /// Read the base id from this path.
    Path(Path),
    /// Always this exact id.
    ///
    /// For example, `gtceu:programmed_circuit`, which isn't written anywhere in a
    /// circuit-configuration slot's own JSON.
    Literal(String),
}

/// A value found alongside a [`MemberCase::Item`]'s base id that should mark it as a different
/// item.
///
/// If present, this is appended to the Id.
///
/// This is useful for, for example, NBT data or programmed circuits (i.e. certain configurable
/// catalysts used in Gregtech recipes).
#[derive(Debug)]
enum IdSuffix {
    /// Append the string at this path verbatim.
    /// Used for NBT. To that end, we just reuse the SNBT string of the kubejs export,
    /// which hopefully is normalised in some way.
    ///
    /// TODO: Check whether this normalisation really holds up. I wouldn't trust code by
    /// default, I would only really trust API contracts in the kubejs documentation (or
    /// its dependencies if kubejs doesn't process it further). Even then, I'm not
    /// comfortable relying on such API specs, because even if kubejs used semver, I have
    /// no guarantees over what kubejs people use to export their recipes. Alternatively,
    /// re-normalise the data using some NBT crate first (though parsing may fail then,
    /// sigh).
    Raw(Path),
    /// Format the integer at this path as `{configuration:N}` and append it.
    ///
    /// Used for `gtceu:circuit` configuration slots, where the base id is a
    /// [`BaseRef::Literal`] (basically we pretend these are new circuit ids), and the
    /// different circuit frequencies are the different configuration numbers.
    Configuration(Path),
}

/// How to resolve one member of an input or output slot list entry.
///
/// Tried in order against one array entry until one resolves, see [`extract_leaf`].
#[derive(Debug)]
enum MemberCase {
    /// A specific item or fluid id, optionally discriminated.
    Item {
        /// Where the base id comes from.
        base: BaseRef,
        /// What, if present, gets appended to the base id.
        id_suffix: Option<IdSuffix>,
    },
    /// A tag id, read from this path.
    Tag(Path),
}

/// How to read a slot's chance from its array entry.
///
/// `numerator / denominator`, with `denominator` defaulting to `default_denominator` if its
/// own path doesn't resolve.
///
/// TODO: Perhaps we should isolate stuff like this into individual functions that the user can
/// _somehow_ call. E.g. Extract a general "division" and a general "default if not present"
/// function. A multiplication and a concatenation funtcion also seem useful. Sigh, this will
/// be a lot of bloat, will it not be?
#[derive(Debug, Deserialize)]
struct ChanceSpec {
    /// Path to the chance numerator.
    numerator: Path,
    /// Path to the chance denominator, if present.
    denominator: Path,
    /// What to use for the denominator when its own path doesn't resolve.
    default_denominator: f32,
}

/// One input or output slot list.
#[derive(Debug)]
struct SlotSpec {
    /// Path to the array of entries. If this doesn't resolve at all, this slot kind
    /// just isn't present on this recipe (e.g. no fluid inputs).
    path: Path,
    /// Path to its count. Defaults to 1 when absent.
    count: Option<Path>,
    /// How to read this entry's chance, if this slot models chance at all.
    chance: Option<ChanceSpec>,
    /// Tried, in order, against an entry. See implementation at [`extract_leaf`].
    members: Vec<MemberCase>,
}

/// A description of where inputs, outputs, duration and EU/t live in one family of recipe
/// JSON.
#[derive(Debug, Default)]
pub(super) struct Shape {
    /// Path to the recipe's duration in ticks, if existing.
    duration: Option<Path>,
    /// Path to the recipe's EU/t, if existing.
    eu_per_tick: Option<Path>,
    /// Every input slot list this shape reads.
    ///
    /// Not optional, but I guess it could be empty?
    inputs: Vec<SlotSpec>,
    /// Every output slot list this shape reads.
    ///
    /// Not optional, but I guess it could be empty?
    outputs: Vec<SlotSpec>,
}

/// Every known shape and which recipes use which.
#[derive(Debug, Default)]
pub(super) struct Shapes {
    /// Shape name -> shape.
    by_name: BTreeMap<String, Shape>,
    /// Exact recipe `type` -> shape name.
    ///
    /// TODO: Clarify what recipe type vs recipe path vs recipe namespace is, both in docs
    /// and for yourself (perhaps this even shows I should expose different selectors
    /// here?)
    ///
    /// Takes priority over `namespaces`.
    types: BTreeMap<String, String>,
    /// Recipe type namespace (the part before `:`) -> shape name.
    ///
    /// Only used when `types` has no exact entry for that recipe's `type`.
    namespaces: BTreeMap<String, String>,
}

/// A problem in a shapes TOML document itself.
#[expect(
    clippy::module_name_repetitions,
    reason = "clearest name for this type, unfortunately."
)]
#[derive(Debug, thiserror::Error)]
pub enum ShapesError {
    /// The document's shape was wrong.
    #[error(transparent)]
    Deserialize(#[from] TomlError),
    /// A `[[shape.*.input]]`/`[[shape.*.output]]` member entry didn't name exactly one
    /// of `item`, `tag`, `literal_item`; or it paired `tag` with a `id_suffix`, which
    /// doesn't apply to tags.
    #[error("{shape}'s {slot_kind}[{slot_index}]'s member[{member_index}]: {reason}")]
    InvalidMember {
        /// The shape this member entry belongs to.
        shape: String,
        /// `"input"` or `"output"`.
        slot_kind: &'static str,
        /// Which slot list within that shape.
        slot_index: usize,
        /// Which member entry within that slot list.
        member_index: usize,
        /// What was wrong with it.
        reason: &'static str,
    },
    /// `recipe_to_shape.types`/`recipe_to_shape.namespaces` named a shape that isn't in
    /// `[shape.*]`.
    #[error("recipe_to_shape names shape {0:?}, which doesn't exist")]
    UnknownShape(String),
}

/// `shapes.toml`'s document.
#[derive(Debug, Deserialize)]
struct RawDocument {
    /// Shape name -> shape.
    #[serde(default)]
    shape: BTreeMap<String, RawShape>,
    /// How recipe types map to shapes.
    #[serde(default)]
    recipe_to_shape: RawRecipeToShape,
}

/// A raw `[shape.*]` table.
#[derive(Debug, Default, Deserialize)]
struct RawShape {
    /// See [`Shape::duration`].
    duration: Option<Path>,
    /// See [`Shape::eu_per_tick`].
    eu_per_tick: Option<Path>,
    /// See [`Shape::inputs`].
    #[serde(default)]
    input: Vec<RawSlotSpec>,
    /// See [`Shape::outputs`].
    #[serde(default)]
    output: Vec<RawSlotSpec>,
}

/// A raw `[[shape.*.input]]`/`[[shape.*.output]]` table.
#[derive(Debug, Deserialize)]
struct RawSlotSpec {
    /// See [`SlotSpec::path`].
    path: Path,
    /// See [`SlotSpec::count`].
    count: Option<Path>,
    /// See [`SlotSpec::chance`].
    chance: Option<ChanceSpec>,
    /// See [`SlotSpec::members`].
    member: Vec<RawMemberCase>,
}

/// A raw `member` entry.
#[derive(Debug, Deserialize)]
struct RawMemberCase {
    /// See [`BaseRef::Path`], for an item/fluid member.
    item: Option<Path>,
    /// A tag member, read from this path.
    tag: Option<Path>,
    /// See [`BaseRef::Literal`], for an item/fluid member.
    literal_item: Option<String>,
    /// See [`IdSuffix`].
    id_suffix: Option<RawIdSuffix>,
}

/// A raw `id_suffix` table.
#[derive(Debug, Deserialize)]
struct RawIdSuffix {
    /// See [`IdSuffix::Raw`].
    raw: Option<Path>,
    /// See [`IdSuffix::Configuration`].
    configuration: Option<Path>,
}

/// A raw `[recipe_to_shape]`.
#[derive(Debug, Default, Deserialize)]
struct RawRecipeToShape {
    /// See [`Shapes::types`].
    #[serde(default)]
    types: BTreeMap<String, String>,
    /// See [`Shapes::namespaces`].
    #[serde(default)]
    namespaces: BTreeMap<String, String>,
}

impl Shapes {
    /// Parses a shapes document.
    ///
    /// # Errors
    ///
    /// If the document's shape is wrong, a member entry is ambiguous or empty, or
    /// `recipe_to_shape` names a shape that doesn't exist.
    pub(super) fn parse(source: &str) -> Result<Self, ShapesError> {
        let document: RawDocument = toml::from_str(source)?;

        let mut shapes = BTreeMap::new();
        for (name, raw) in document.shape {
            let inputs = raw
                .input
                .into_iter()
                .enumerate()
                .map(|(index, slot)| resolve_slot_spec(&name, "input", index, slot))
                .collect::<Result<_, _>>()?;
            let outputs = raw
                .output
                .into_iter()
                .enumerate()
                .map(|(index, slot)| resolve_slot_spec(&name, "output", index, slot))
                .collect::<Result<_, _>>()?;
            let _: Option<Shape> = shapes.insert(
                name,
                Shape {
                    duration: raw.duration,
                    eu_per_tick: raw.eu_per_tick,
                    inputs,
                    outputs,
                },
            );
        }

        for shape_name in document
            .recipe_to_shape
            .types
            .values()
            .chain(document.recipe_to_shape.namespaces.values())
        {
            if !shapes.contains_key(shape_name) {
                return Err(ShapesError::UnknownShape(shape_name.clone()));
            }
        }

        Ok(Self {
            by_name: shapes,
            types: document.recipe_to_shape.types,
            namespaces: document.recipe_to_shape.namespaces,
        })
    }

    /// This crate's reference shapes ([`crate::ingest::DEFAULT_SHAPES_TOML`]).
    ///
    /// Only parsed once, and only useful for tests.
    ///
    /// TODO: Is it tidy enough to keep this here then, instead of moving it to the test
    /// helpers collection?
    ///
    /// # Panics
    ///
    /// Never, unless `shapes.toml` itself is broken, which would be raised during the tests.
    #[cfg(test)]
    pub(super) fn builtin() -> &'static Self {
        static SHAPES: OnceLock<Shapes> = OnceLock::new();
        SHAPES.get_or_init(|| {
            Self::parse(DEFAULT_SHAPES_TOML)
                .expect("shapes.toml is valid, as checked by its own test")
        })
    }

    /// The shape to use for `recipe_type`, if any.
    ///
    /// An exact match in `types`, else a match on `recipe_type`'s namespace in `namespaces`.
    /// If neither is found, this should be surfaced as a warning to the user.
    pub(super) fn shape_for(&self, recipe_type: &str) -> Option<&Shape> {
        let shape_name = self.types.get(recipe_type).or_else(|| {
            let namespace = recipe_type
                .split_once(':')
                .map_or(recipe_type, |(namespace, _)| namespace);
            self.namespaces.get(namespace)
        })?;
        self.by_name.get(shape_name)
    }
}

/// Turns one raw `member` entry into a [`MemberCase`].
fn resolve_member_case(
    shape: &str,
    slot_kind: &'static str,
    slot_index: usize,
    member_index: usize,
    raw: RawMemberCase,
) -> Result<MemberCase, ShapesError> {
    let error = |reason| ShapesError::InvalidMember {
        shape: shape.to_owned(),
        slot_kind,
        slot_index,
        member_index,
        reason,
    };

    if let Some(path) = raw.tag {
        return if raw.item.is_some() || raw.literal_item.is_some() || raw.id_suffix.is_some() {
            Err(error(
                "`tag` can't be combined with `item`, `literal_item` or `id_suffix`",
            ))
        } else {
            Ok(MemberCase::Tag(path))
        };
    }

    let base = match (raw.item, raw.literal_item) {
        (Some(path), None) => BaseRef::Path(path),
        (None, Some(literal)) => BaseRef::Literal(literal),
        (None, None) => {
            return Err(error("needs exactly one of `item`, `tag`, `literal_item`"));
        }
        (Some(_), Some(_)) => {
            return Err(error("`item` and `literal_item` are mutually exclusive"));
        }
    };

    let id_suffix = match raw.id_suffix {
        None => None,
        Some(RawIdSuffix {
            raw: Some(path),
            configuration: None,
        }) => Some(IdSuffix::Raw(path)),
        Some(RawIdSuffix {
            raw: None,
            configuration: Some(path),
        }) => Some(IdSuffix::Configuration(path)),
        Some(RawIdSuffix {
            raw: None,
            configuration: None,
        }) => return Err(error("`id_suffix` needs one of `raw`, `configuration`")),
        Some(RawIdSuffix {
            raw: Some(_),
            configuration: Some(_),
        }) => {
            return Err(error(
                "`id_suffix.raw` and `id_suffix.configuration` are mutually exclusive",
            ));
        }
    };

    Ok(MemberCase::Item { base, id_suffix })
}

/// Turns one raw `[[shape.*.input]]`/`[[shape.*.output]]` table into a [`SlotSpec`].
fn resolve_slot_spec(
    shape: &str,
    slot_kind: &'static str,
    slot_index: usize,
    raw: RawSlotSpec,
) -> Result<SlotSpec, ShapesError> {
    let members = raw
        .member
        .into_iter()
        .enumerate()
        .map(|(member_index, member)| {
            resolve_member_case(shape, slot_kind, slot_index, member_index, member)
        })
        .collect::<Result<_, _>>()?;
    Ok(SlotSpec {
        path: raw.path,
        count: raw.count,
        chance: raw.chance,
        members,
    })
}

/// Tries each of `members` against `node` in order, returning the first that resolves.
///
/// For [`MemberCase::Item`], an [`IdSuffix`] that doesn't resolve on `node` just means this
/// particular entry doesn't have one. In this case, we use the base id unsuffixed, and this is
/// not treated as an error. In fact, most recipes in modern Gregtech do not have NBT.
fn extract_leaf(node: &Value, members: &[MemberCase]) -> Option<Leaf> {
    for member in members {
        match member {
            MemberCase::Tag(path) => {
                if let Some(tag) = path.resolve(node).and_then(Value::as_str) {
                    return Some(Leaf::Tag(tag.to_owned()));
                }
            }
            MemberCase::Item { base, id_suffix } => {
                let maybe_base = match base {
                    BaseRef::Path(path) => path
                        .resolve(node)
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    BaseRef::Literal(id) => Some(id.clone()),
                };
                let Some(base_id) = maybe_base else {
                    continue;
                };
                let id = fold_id_suffix(base_id, id_suffix.as_ref(), node);
                return Some(Leaf::Item(id));
            }
        }
    }
    None
}

/// Appends `id_suffix`'s value onto `base_id`, if it resolves on `node`.
fn fold_id_suffix(base_id: String, id_suffix: Option<&IdSuffix>, node: &Value) -> String {
    match id_suffix {
        None => base_id,
        Some(IdSuffix::Raw(path)) => match path.resolve(node).and_then(Value::as_str) {
            Some(suffix) => format!("{base_id}{suffix}"),
            None => base_id,
        },
        Some(IdSuffix::Configuration(path)) => {
            match path.resolve(node).and_then(Value::as_u64) {
                Some(configuration) => format!("{base_id}{{configuration:{configuration}}}"),
                None => base_id,
            }
        }
    }
}

/// Reads `path`'s value on `node` as a count, defaulting to 1 when `path` is absent or doesn't
/// resolve.
///
/// TODO: Should this not maybe sometimes be a warning if it doesn't resolve? Perhaps it'd be
/// better to let the user decide, with the same method mentioned in the other TODO, about
/// extracting some kind of "this is optional, default to that" primitive, vs "i want this key
/// and if it's absent that's an error" kind of deal?
fn resolve_count(maybe_path: Option<&Path>, node: &Value) -> Result<u32, String> {
    let Some(count_path) = maybe_path else {
        return Ok(1);
    };
    count_path.resolve(node).map_or(Ok(1), |value| {
        value
            .as_u64()
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| "count is present but not a u32".to_owned())
    })
}

/// Reads an entry's chance per `spec`, defaulting to 1.0 (always consumed/produced) when
/// `spec` is absent, or its numerator path doesn't resolve on `node`.
///
/// TODO: See the todo on `resolve_count`.
fn resolve_chance(maybe_spec: Option<&ChanceSpec>, node: &Value) -> Result<f32, String> {
    let Some(chance_spec) = maybe_spec else {
        return Ok(1.0);
    };
    let Some(numerator) = chance_spec.numerator.resolve(node).and_then(Value::as_f64) else {
        return Ok(1.0);
    };
    let denominator = chance_spec
        .denominator
        .resolve(node)
        .and_then(Value::as_f64)
        .unwrap_or_else(|| f64::from(chance_spec.default_denominator));
    if denominator == 0.0 {
        return Err("chance denominator is zero".to_owned());
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "f64 -> f32 chance ratio; GTCEu's chance/maxChance are small near-integer \
                  values, the precision loss is irrelevant"
    )]
    let ratio = (numerator / denominator) as f32;
    super::validate_chance(ratio)
}

/// Reads the entries a [`SlotSpec`]'s path resolved to.
///
/// Reads an array as its elements, and reads a single object as a one-element list (some
/// shapes have exactly one not-wrapped result instead of a one-element array, e.g. EnderIO's
/// bare `result` object).
fn as_entries(node: &Value) -> Result<Vec<&Value>, String> {
    match node {
        Value::Array(entries) => Ok(entries.iter().collect()),
        Value::Object(_) => Ok(vec![node]),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            Err("expected an array or object".to_owned())
        }
    }
}

/// Extracts every slot this [`SlotSpec`] describes as input [`Slot`]s.
///
/// An empty result (not an error) if `spec.path` doesn't resolve at all, because it just means
/// this slot kind isn't present on this recipe (e.g. no fluid inputs).
fn extract_input_slots(
    spec: &SlotSpec,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<Vec<Slot>, String> {
    let Some(node) = spec.path.resolve(value) else {
        return Ok(Vec::new());
    };
    let entries = as_entries(node)?;

    let mut slots = Vec::with_capacity(entries.len());
    for entry in entries {
        let leaf = extract_leaf(entry, &spec.members)
            .ok_or_else(|| "unrecognized member shape".to_owned())?;
        let leaves = [leaf];
        let count = resolve_count(spec.count.as_ref(), entry)?;
        let chance = resolve_chance(spec.chance.as_ref(), entry)?;
        let members = resolve_leaves(&leaves, resolver)?;
        let tag = slot_tag(&leaves, resolver);
        slots.push(Slot::new(members, tag, count, chance));
    }
    Ok(slots)
}

/// Extracts every slot this [`SlotSpec`] describes as [`Output`]s.
///
/// An empty result (not an error) if `spec.path` doesn't resolve at all, same as
/// [`extract_input_slots`].
fn extract_output_slots(
    spec: &SlotSpec,
    value: &Value,
    resolver: &Resolver<'_>,
) -> Result<Vec<Output>, String> {
    let Some(node) = spec.path.resolve(value) else {
        return Ok(Vec::new());
    };
    let entries = as_entries(node)?;

    let mut outputs = Vec::with_capacity(entries.len());
    for entry in entries {
        let leaf = extract_leaf(entry, &spec.members)
            .ok_or_else(|| "unrecognized member shape".to_owned())?;
        let Leaf::Item(id) = leaf else {
            return Err("an output resolved to a tag, not a concrete item".to_owned());
        };
        let item = resolve_output_id(&id, resolver);
        let count = resolve_count(spec.count.as_ref(), entry)?;
        let chance = resolve_chance(spec.chance.as_ref(), entry)?;
        outputs.push(Output::new(item, count, chance));
    }
    Ok(outputs)
}

/// Applies `shape` to one recipe, producing its draft.
pub(super) fn apply(
    id: String,
    value: &Value,
    shape: &Shape,
    resolver: &Resolver<'_>,
) -> Result<RecipeDraft, String> {
    let mut inputs = Vec::new();
    for spec in &shape.inputs {
        inputs.extend(extract_input_slots(spec, value, resolver)?);
    }
    let mut outputs = Vec::new();
    for spec in &shape.outputs {
        outputs.extend(extract_output_slots(spec, value, resolver)?);
    }

    let ticks = shape
        .duration
        .as_ref()
        .and_then(|path| path.resolve(value))
        .and_then(Value::as_u64)
        .and_then(|ticks| u32::try_from(ticks).ok());
    let eu_t = shape
        .eu_per_tick
        .as_ref()
        .and_then(|path| path.resolve(value))
        .and_then(Value::as_i64);

    Ok(RecipeDraft {
        id,
        inputs,
        outputs,
        eu_t,
        ticks,
    })
}

#[cfg(test)]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "these item ids deliberately contain `{...}` NBT id_suffix suffixes, \
              these are not format arguments"
)]
mod tests {
    use core::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::Shapes;
    use crate::ingest::{ItemInterner, PlainResolverParts, Resolver};
    use crate::model::ItemIndex;

    #[test]
    fn builtin_shapes_parse_without_error() {
        let _: &Shapes = Shapes::builtin();
    }

    /// `gtceu:macerator`, the real `macerate_hv_stainless_steel_buzzsaw` shape minus the
    /// NBT tool gate (covered separately below).
    #[test]
    fn gtceu_machine_shape_reads_a_real_macerator_recipe() {
        let shapes = Shapes::builtin();
        let shape = shapes
            .shape_for("gtceu:macerator")
            .expect("gtceu:macerator is recipe_to_shape'ed to a shape");

        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let value = json!({
            "type": "gtceu:macerator",
            "duration": 400,
            "tickInputs": {"eu": [{"content": 8}]},
            "inputs": {
                "item": [
                    {"content": {"type": "gtceu:sized", "count": 1, "ingredient": {"item": "gtceu:iron_ore"}}}
                ]
            },
            "outputs": {
                "item": [
                    {"content": {"type": "gtceu:sized", "count": 2, "ingredient": {"item": "gtceu:iron_dust"}}},
                    {"content": {"type": "gtceu:sized", "count": 1, "ingredient": {"item": "gtceu:stone_dust"}}, "chance": 6000, "maxChance": 10000}
                ]
            }
        });

        let draft =
            super::apply("test".to_owned(), &value, shape, &resolver).expect("valid recipe");

        assert_eq!(draft.ticks, Some(400));
        assert_eq!(draft.eu_t, Some(8));
        assert_eq!(draft.inputs.len(), 1);
        assert_eq!(draft.outputs.len(), 2);
        let guaranteed_output = draft.outputs.first().expect("two outputs");
        let chanced_output = draft.outputs.get(1).expect("two outputs");
        assert_eq!(guaranteed_output.count(), 2);
        assert!(
            (guaranteed_output.chance() - 1.0).abs() < f32::EPSILON,
            "no chance field present -> always produced"
        );
        assert!(
            (chanced_output.chance() - 0.6).abs() < 1e-4,
            "6000/10000 -> 0.6, got {}",
            chanced_output.chance()
        );
    }

    /// `gtceu:distillery`'s real fluid-in and fluid-out shape, plus a chance-0 circuit
    /// configuration slot.
    ///
    /// This tests that catalysts fall out of `chance == 0` automatically and that
    /// `gtceu:circuit` resolves to the same item as `gtceu:programmed_circuit` but
    /// suffixed by its configuration.
    #[test]
    fn gtceu_machine_shape_reads_circuits_and_fluids() {
        let shapes = Shapes::builtin();
        let shape = shapes
            .shape_for("gtceu:distillery")
            .expect("gtceu:distillery is recipe_to_shape'ed to a shape");

        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let value = json!({
            "type": "gtceu:distillery",
            "duration": 600,
            "tickInputs": {"eu_per_tick": [{"content": 120}]},
            "inputs": {
                "item": [
                    {"content": {"type": "gtceu:circuit", "configuration": 1}, "chance": 0}
                ],
                "fluid": [
                    {"content": {"amount": 2000, "value": [{"fluid": "gtceu:naquadria_waste"}]}}
                ]
            },
            "outputs": {
                "item": [
                    {"content": {"type": "gtceu:sized", "count": 1, "ingredient": {"item": "gtceu:gallium_sulfide_dust"}}}
                ],
                "fluid": [
                    {"content": {"amount": 500, "value": [{"fluid": "gtceu:sulfuric_acid"}]}}
                ]
            }
        });

        let draft =
            super::apply("test".to_owned(), &value, shape, &resolver).expect("valid recipe");

        // One item input (the circuit) and one fluid input.
        assert_eq!(draft.inputs.len(), 2);
        let circuit_slot = draft
            .inputs
            .iter()
            .find(|slot| slot.tag().is_none())
            .expect("the circuit slot has no tag");
        assert!(
            (circuit_slot.chance() - 0.0).abs() < f32::EPSILON,
            "chance 0 makes the circuit a catalyst"
        );
        let expected_circuit = parts
            .interner
            .borrow_mut()
            .intern("gtceu:programmed_circuit{configuration:1}");
        assert_eq!(
            circuit_slot.members().iter().next().copied(),
            Some(expected_circuit),
            "circuit configuration 1 resolves to the real item, suffixed"
        );

        assert_eq!(draft.outputs.len(), 2);
    }

    /// NBT folds into the id only when present, and is appended verbatim.
    #[test]
    fn gtceu_machine_shape_folds_nbt_into_the_item_id() {
        let shapes = Shapes::builtin();
        let shape = shapes
            .shape_for("gtceu:macerator")
            .expect("gtceu:macerator is recipe_to_shape'ed to a shape");

        let parts = PlainResolverParts::default();
        let resolver = parts.resolver();

        let value = json!({
            "type": "gtceu:macerator",
            "duration": 1323,
            "tickInputs": {"eu_per_tick": [{"content": 8}]},
            "inputs": {
                "item": [
                    {"content": {"type": "gtceu:sized", "count": 1, "ingredient": {
                        "type": "forge:nbt",
                        "item": "gtceu:hv_stainless_steel_buzzsaw",
                        "count": 1,
                        "nbt": "{Damage:0,GT.Tool:{}}"
                    }}}
                ]
            },
            "outputs": {}
        });

        let draft =
            super::apply("test".to_owned(), &value, shape, &resolver).expect("valid recipe");

        assert_eq!(draft.inputs.len(), 1);
        let input_slot = draft.inputs.first().expect("one input");
        let expected = parts
            .interner
            .borrow_mut()
            .intern("gtceu:hv_stainless_steel_buzzsaw{Damage:0,GT.Tool:{}}");
        assert_eq!(input_slot.members().iter().next().copied(), Some(expected));
    }

    /// A tag reference, with no `count` or `chance` fields at all, so defaults are applied.
    #[test]
    fn gtceu_machine_shape_defaults_absent_count_and_chance() {
        let shapes = Shapes::builtin();
        let shape = shapes
            .shape_for("gtceu:arc_furnace")
            .expect("gtceu:arc_furnace is recipe_to_shape'ed to a shape");

        let tag_set: BTreeSet<ItemIndex> = BTreeSet::new();
        let tag_members: BTreeMap<String, &BTreeSet<ItemIndex>> =
            BTreeMap::from([("forge:double_plates/rose_gold".to_owned(), &tag_set)]);
        let tag_index: BTreeMap<String, u32> =
            BTreeMap::from([("forge:double_plates/rose_gold".to_owned(), 0)]);
        let interner = RefCell::new(ItemInterner::default());
        let direct_refs = RefCell::new(BTreeSet::new());
        let used_tags = RefCell::new(BTreeSet::new());
        let resolver = Resolver {
            tag_index: &tag_index,
            tag_members: &tag_members,
            interner: &interner,
            direct_refs: &direct_refs,
            used_tags: &used_tags,
        };

        let value = json!({
            "type": "gtceu:arc_furnace",
            "duration": 338,
            "inputs": {
                "item": [
                    {"content": {"type": "gtceu:sized", "count": 1, "ingredient": {"tag": "forge:double_plates/rose_gold"}}}
                ]
            },
            "outputs": {
                "item": [
                    {"content": {"type": "gtceu:sized", "count": 2, "ingredient": {"item": "gtceu:rose_gold_ingot"}}}
                ]
            }
        });

        let draft =
            super::apply("test".to_owned(), &value, shape, &resolver).expect("valid recipe");

        assert_eq!(draft.eu_t, None, "no tickInputs.eu at all on this recipe");
        let output = draft.outputs.first().expect("one output");
        assert!((output.chance() - 1.0).abs() < f32::EPSILON);
    }
}
