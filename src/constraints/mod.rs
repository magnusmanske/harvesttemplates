//! Property constraint checks before an edit. Most run on the freshly fetched
//! item; the rest ask WDQS, Commons or search. See `docs/CONSTRAINTS.md`.

mod local;
mod remote;

use crate::ids::{ItemId, PropertyId};
use crate::value::{Datatype, Value};
use crate::wiki::MwApi;
use crate::wikidata::{ConstraintDef, Entity, Wdqs};
use anyhow::Result;
use remote::Remote;
use serde::Serialize;

const EXCEPTIONS: PropertyId = PropertyId(2303);

/// The statement about to be added, and the item it goes to.
#[derive(Debug)]
pub struct Candidate<'a> {
    pub item: &'a Entity,
    pub property: PropertyId,
    pub datatype: Datatype,
    pub value: &'a Value,
    pub qualifiers: &'a [PropertyId],
}

/// External services some checks need.
#[derive(Debug, Clone, Copy)]
pub struct Services<'a> {
    pub wdqs: &'a Wdqs,
    pub mw: &'a MwApi,
}

#[derive(Debug, Clone, Copy)]
enum Rule {
    /// Returns `true` on violation.
    Local(fn(&ConstraintDef, &Candidate) -> bool),
    Remote(Remote),
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ConstraintType {
    pub id: ItemId,
    pub name: &'static str,
    #[serde(skip)]
    rule: Rule,
}

const fn local(
    id: u64,
    name: &'static str,
    f: fn(&ConstraintDef, &Candidate) -> bool,
) -> ConstraintType {
    ConstraintType {
        id: ItemId(id),
        name,
        rule: Rule::Local(f),
    }
}

const fn remote(id: u64, name: &'static str, r: Remote) -> ConstraintType {
    ConstraintType {
        id: ItemId(id),
        name,
        rule: Rule::Remote(r),
    }
}

/// Every supported constraint type.
pub static TYPES: [ConstraintType; 23] = [
    local(
        52_004_125,
        "allowed entity types",
        local::allowed_entity_types,
    ),
    local(21_510_851, "allowed qualifiers", local::allowed_qualifiers),
    local(21_514_353, "allowed units", local::allowed_units),
    local(54_554_025, "citation needed", local::citation_needed),
    remote(21_510_852, "Commons link", Remote::CommonsLink),
    remote(21_502_838, "conflicts with", Remote::ConflictsWith),
    remote(21_502_410, "distinct values", Remote::Distinct),
    local(21_502_404, "format", local::format),
    local(52_848_401, "integer", local::integer),
    remote(21_510_855, "inverse", Remote::Inverse),
    local(
        21_503_247,
        "item requires statement",
        local::item_requires_statement,
    ),
    local(
        21_510_856,
        "mandatory qualifier",
        local::mandatory_qualifier,
    ),
    local(51_723_761, "no bounds", local::no_bounds),
    local(52_558_054, "none of", local::none_of),
    local(21_510_859, "one of", local::one_of),
    local(53_869_507, "property scope", local::property_scope),
    local(21_510_860, "range", local::range),
    local(19_474_404, "single value", local::single_value),
    local(52_060_874, "single best value", local::single_best_value),
    remote(21_510_862, "symmetric", Remote::Symmetric),
    remote(21_503_250, "type", Remote::Type),
    remote(
        21_510_864,
        "value requires statement",
        Remote::ValueRequiresStatement,
    ),
    remote(21_510_865, "value type", Remote::ValueType),
];

pub fn find(id: ItemId) -> Option<&'static ConstraintType> {
    TYPES.iter().find(|t| t.id == id)
}

/// The name of the first violated constraint among `defs`, if any.
/// Unsupported constraint types are ignored; the UI lists them as such.
pub async fn first_violation(
    defs: &[&ConstraintDef],
    candidate: &Candidate<'_>,
    services: Services<'_>,
) -> Result<Option<&'static str>> {
    for def in defs {
        let Some(kind) = find(def.kind) else { continue };
        if def.items(EXCEPTIONS).contains(&candidate.item.id) {
            continue;
        }
        let violated = match kind.rule {
            Rule::Local(f) => f(def, candidate),
            Rule::Remote(r) => r.violated(def, candidate, services).await?,
        };
        if violated {
            return Ok(Some(kind.name));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_ids_are_unique() {
        let ids: HashSet<_> = TYPES.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), TYPES.len());
        assert_eq!(find(ItemId(21_502_410)).unwrap().name, "distinct values");
        assert!(find(ItemId(1)).is_none());
    }
}
