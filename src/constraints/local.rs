//! Checks that only need the constraint definition, the item and the new value.
//! Each returns `true` on violation.

use super::Candidate;
use crate::ids::{ItemId, PropertyId};
use crate::value::{Date, Value, existing_date};
use crate::wikidata::ConstraintDef;
use fancy_regex::{Regex, RegexBuilder};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub const ITEM_OF_CONSTRAINT: PropertyId = PropertyId(2305);
pub const PROPERTY: PropertyId = PropertyId(2306);
const FORMAT: PropertyId = PropertyId(1793);
const MIN_DATE: PropertyId = PropertyId(2310);
const MAX_DATE: PropertyId = PropertyId(2311);
const MAX_QUANTITY: PropertyId = PropertyId(2312);
const MIN_QUANTITY: PropertyId = PropertyId(2313);
const SCOPE: PropertyId = PropertyId(5314);
const WIKIBASE_ITEM: ItemId = ItemId(29_934_200);
const AS_MAIN_VALUE: ItemId = ItemId(54_828_448);
/// Keeps a pathological format pattern from tying up a worker.
const BACKTRACK_LIMIT: usize = 100_000;

pub fn allowed_entity_types(def: &ConstraintDef, _: &Candidate) -> bool {
    let types = def.items(ITEM_OF_CONSTRAINT);
    !types.is_empty() && !types.contains(&WIKIBASE_ITEM)
}

pub fn allowed_qualifiers(def: &ConstraintDef, c: &Candidate) -> bool {
    let allowed: Vec<PropertyId> = def.entity_ids(PROPERTY).iter().filter_map(|p| p.parse().ok()).collect();
    c.qualifiers.iter().any(|q| !allowed.contains(q))
}

pub fn allowed_units(def: &ConstraintDef, c: &Candidate) -> bool {
    let Value::Quantity { unit, .. } = c.value else {
        return false;
    };
    let allowed: Vec<Option<ItemId>> = def
        .snaks(ITEM_OF_CONSTRAINT)
        .iter()
        .map(|s| s["datavalue"]["value"]["id"].as_str().and_then(|id| id.parse().ok()))
        .collect();
    !allowed.contains(unit)
}

/// Every statement we add carries a reference.
pub const fn citation_needed(_: &ConstraintDef, _: &Candidate) -> bool {
    false
}

pub fn format(def: &ConstraintDef, c: &Candidate) -> bool {
    let text = match c.value {
        Value::String(s) | Value::Monolingual { text: s, .. } => s,
        _ => return false,
    };
    let Some(pattern) = def.first_string(FORMAT) else {
        return false;
    };
    let Some(regex) = compiled(pattern) else {
        return false;
    };
    match regex.is_match(text) {
        Ok(matched) => !matched,
        Err(e) => {
            tracing::warn!("format check gave up on /{pattern}/: {e}");
            false
        }
    }
}

/// Wikidata format patterns are PCRE; fancy-regex covers what they use. Compiled once per pattern.
fn compiled(pattern: &str) -> Option<Arc<Regex>> {
    static CACHE: LazyLock<Mutex<HashMap<String, Option<Arc<Regex>>>>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(pattern.to_string())
        .or_insert_with(|| {
            let anchored = format!("^(?:{pattern})$");
            RegexBuilder::new(&anchored)
                .backtrack_limit(BACKTRACK_LIMIT)
                .build()
                .inspect_err(|e| tracing::warn!("unsupported format pattern /{pattern}/: {e}"))
                .ok()
                .map(Arc::new)
        })
        .clone()
}

pub fn integer(_: &ConstraintDef, c: &Candidate) -> bool {
    let Value::Quantity { amount, .. } = c.value else {
        return false;
    };
    amount.split_once('.').is_some_and(|(_, fraction)| !fraction.trim_end_matches('0').is_empty())
}

pub fn item_requires_statement(def: &ConstraintDef, c: &Candidate) -> bool {
    let Some(required) = def.first_property(PROPERTY) else {
        return false;
    };
    let values = def.items(ITEM_OF_CONSTRAINT);
    !has_statement(c, required, &values)
}

/// Does the item have `property`, with one of `values` if any are given?
pub fn has_statement(c: &Candidate, property: PropertyId, values: &[ItemId]) -> bool {
    match values {
        [] => c.item.has_property(property),
        _ => c.item.item_values(property).iter().any(|v| values.contains(v)),
    }
}

pub fn mandatory_qualifier(def: &ConstraintDef, c: &Candidate) -> bool {
    let required = def.entity_ids(PROPERTY);
    required.iter().filter_map(|p| p.parse().ok()).any(|p| !c.qualifiers.contains(&p))
}

/// We never add bounds.
pub const fn no_bounds(_: &ConstraintDef, _: &Candidate) -> bool {
    false
}

pub fn none_of(def: &ConstraintDef, c: &Candidate) -> bool {
    matches!(c.value, Value::Item(q) if def.items(ITEM_OF_CONSTRAINT).contains(q))
}

pub fn one_of(def: &ConstraintDef, c: &Candidate) -> bool {
    matches!(c.value, Value::Item(q) if !def.items(ITEM_OF_CONSTRAINT).contains(q))
}

pub fn property_scope(def: &ConstraintDef, _: &Candidate) -> bool {
    let scopes = def.items(SCOPE);
    !scopes.is_empty() && !scopes.contains(&AS_MAIN_VALUE)
}

pub fn range(def: &ConstraintDef, c: &Candidate) -> bool {
    match c.value {
        Value::Quantity { amount, .. } => {
            let amount: f64 = amount.parse().unwrap_or(f64::NAN);
            let bound = |p| bound(def, p).and_then(|v| v["amount"].as_str()?.parse::<f64>().ok());
            bound(MIN_QUANTITY).is_some_and(|min| amount < min) || bound(MAX_QUANTITY).is_some_and(|max| amount > max)
        }
        Value::Time { date, .. } => {
            let bound = |p| date_bound(def, p);
            bound(MIN_DATE).is_some_and(|min| date.latest() < min)
                || bound(MAX_DATE).is_some_and(|max| date.earliest() > max)
        }
        _ => false,
    }
}

/// The value of a bound qualifier; `None` for "no value" (unbounded).
fn bound(def: &ConstraintDef, property: PropertyId) -> Option<&Json> {
    def.snaks(property).first().filter(|s| s["snaktype"] == "value").map(|s| &s["datavalue"]["value"])
}

/// "Unknown value" as a date bound means "now".
fn date_bound(def: &ConstraintDef, property: PropertyId) -> Option<Date> {
    let snak = def.snaks(property).first()?;
    if snak["snaktype"] == "somevalue" {
        let today = chrono::Utc::now().date_naive();
        use chrono::Datelike;
        return Some(Date { year: today.year().into(), month: today.month() as u8, day: today.day() as u8 });
    }
    existing_date(bound(def, property)?).map(|d| d.earliest())
}

pub fn single_value(_: &ConstraintDef, c: &Candidate) -> bool {
    c.item.statements(c.property).iter().any(|s| s["rank"] != "deprecated")
}

/// Adding a normal-rank value next to existing normal-rank ones (and no
/// preferred one) leaves several best values.
pub fn single_best_value(_: &ConstraintDef, c: &Candidate) -> bool {
    let ranks: Vec<&str> = c.item.statements(c.property).iter().filter_map(|s| s["rank"].as_str()).collect();
    ranks.contains(&"normal") && !ranks.contains(&"preferred")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Calendar, Datatype};
    use crate::wikidata::Entity;
    use serde_json::json;

    fn def(kind: &str, qualifiers: Json) -> ConstraintDef {
        let statement = json!({
            "mainsnak": {"datavalue": {"value": {"id": kind}}},
            "qualifiers": qualifiers,
        });
        ConstraintDef::from_statement(&statement).unwrap()
    }

    fn item_snaks(ids: &[&str]) -> Json {
        ids.iter().map(|id| json!({"snaktype": "value", "datavalue": {"value": {"id": id}}})).collect()
    }

    fn string_snak(s: &str) -> Json {
        json!([{"snaktype": "value", "datavalue": {"value": s}}])
    }

    fn check(f: fn(&ConstraintDef, &Candidate) -> bool, d: &ConstraintDef, item: &Json, value: Value) -> bool {
        let entity = Entity::new(item.clone()).unwrap();
        let datatype = match value {
            Value::Quantity { .. } => Datatype::Quantity,
            Value::Time { .. } => Datatype::Time,
            Value::Item(_) => Datatype::Item,
            _ => Datatype::String,
        };
        let c = Candidate { item: &entity, property: PropertyId(1), datatype, value: &value, qualifiers: &[] };
        f(d, &c)
    }

    fn item(claims: Json) -> Json {
        json!({"id": "Q1", "claims": claims})
    }

    #[test]
    fn format_checks() {
        let d = def("Q21502404", json!({"P1793": string_snak(r"tt\d{7,8}")}));
        let empty = item(json!({}));
        assert!(!check(format, &d, &empty, Value::String("tt0111161".into())));
        assert!(check(format, &d, &empty, Value::String("tt0111161x".into())), "anchored");
        let d = def("Q21502404", json!({"P1793": string_snak(r"(?i)[a-z]+(?=\d)\d")}));
        assert!(!check(format, &d, &empty, Value::String("AB1".into())), "flags and lookahead (#161)");
        let d = def("Q21502404", json!({"P1793": string_snak(r"(unclosed")}));
        assert!(!check(format, &d, &empty, Value::String("x".into())), "broken patterns don't block");
    }

    #[test]
    fn value_lists() {
        let d = def("Q21510859", json!({"P2305": item_snaks(&["Q5", "Q6"])}));
        let empty = item(json!({}));
        assert!(!check(one_of, &d, &empty, Value::Item(ItemId(5))));
        assert!(check(one_of, &d, &empty, Value::Item(ItemId(7))));
        assert!(check(none_of, &d, &empty, Value::Item(ItemId(6))));
    }

    #[test]
    fn units_and_integers() {
        let novalue = json!({"snaktype": "novalue"});
        let d = def("Q21514353", json!({"P2305": [novalue]}));
        let empty = item(json!({}));
        let qty = |amount: &str, unit| Value::Quantity { amount: amount.into(), unit };
        assert!(!check(allowed_units, &d, &empty, qty("+62", None)), "#178: no unit allowed");
        assert!(check(allowed_units, &d, &empty, qty("+62", Some(ItemId(11_573)))));
        assert!(!check(integer, &d, &empty, qty("+62.000", None)));
        assert!(check(integer, &d, &empty, qty("+62.5", None)));
    }

    #[test]
    fn ranges() {
        let amount = |a: &str| json!([{"snaktype": "value", "datavalue": {"value": {"amount": a}}}]);
        let d = def("Q21510860", json!({"P2313": amount("+0"), "P2312": amount("+100"), }));
        let empty = item(json!({}));
        let qty = |a: &str| Value::Quantity { amount: a.into(), unit: None };
        assert!(!check(range, &d, &empty, qty("+50")));
        assert!(check(range, &d, &empty, qty("-1")));
        let time = |t: &str| json!([{"snaktype": "value", "datavalue": {"value": {"time": t, "precision": 9}}}]);
        let d = def("Q21510860", json!({"P2310": time("+1800-00-00T00:00:00Z"), "P2311": [{"snaktype": "somevalue"}]}));
        let date = |year| Value::Time { date: Date { year, month: 0, day: 0 }, calendar: Calendar::Gregorian };
        assert!(!check(range, &d, &empty, date(1950)));
        assert!(check(range, &d, &empty, date(1700)));
        assert!(check(range, &d, &empty, date(3000)), "unknown max means now");
    }

    #[test]
    fn single_values() {
        let d = def("Q19474404", json!({}));
        let with = item(json!({"P1": [{"rank": "normal", "mainsnak": {}}]}));
        let deprecated = item(json!({"P1": [{"rank": "deprecated", "mainsnak": {}}]}));
        let preferred = item(json!({"P1": [{"rank": "preferred"}, {"rank": "normal"}]}));
        let v = || Value::String("x".into());
        assert!(check(single_value, &d, &with, v()));
        assert!(!check(single_value, &d, &deprecated, v()));
        assert!(check(single_best_value, &d, &with, v()));
        assert!(!check(single_best_value, &d, &preferred, v()));
    }

    #[test]
    fn item_requires_statements() {
        let d = def("Q21503247", json!({"P2306": item_snaks(&["P31"]), "P2305": item_snaks(&["Q5"])}));
        let human = item(json!({"P31": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"id": "Q5"}}}}]}));
        let cat = item(json!({"P31": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"id": "Q146"}}}}]}));
        assert!(!check(item_requires_statement, &d, &human, Value::String("x".into())));
        assert!(check(item_requires_statement, &d, &cat, Value::String("x".into())));
    }
}
