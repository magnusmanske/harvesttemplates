use crate::ids::{ItemId, PropertyId};
use crate::value::{Datatype, Date, Value, existing_date};
use crate::wiki::MwApi;
use crate::wiki::api::params;
use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value as Json};
use std::collections::HashMap;

pub const HOST: &str = "www.wikidata.org";
const IDS_PER_REQUEST: usize = 50;

const PROPERTY_CONSTRAINT: PropertyId = PropertyId(2302);
const CONSTRAINT_STATUS: PropertyId = PropertyId(2316);
const MANDATORY: ItemId = ItemId(21_502_408);
const SUGGESTION: ItemId = ItemId(62_026_391);
const DEPRECATED_PROPERTY_CLASSES: [ItemId; 2] = [ItemId(37_911_748), ItemId(18_644_427)];
const INSTANCE_OF: PropertyId = PropertyId(31);
const UNIT_SYMBOL: PropertyId = PropertyId(5061);
const FORMATTER_URL: PropertyId = PropertyId(1630);
const ALLOWED_UNITS: ItemId = ItemId(21_514_353);
const ITEM_OF_CONSTRAINT: PropertyId = PropertyId(2305);

/// Read access to Wikidata entities.
#[derive(Debug, Clone)]
pub struct Wikidata {
    pub api: MwApi,
}

impl Wikidata {
    /// Raw entity JSON for `ids` (any entity type), in batches of 50.
    pub async fn entities(&self, ids: &[String], props: &str) -> Result<Vec<Json>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(IDS_PER_REQUEST) {
            let ids = chunk.join("|");
            let p = params(&[("action", "wbgetentities"), ("ids", &ids), ("props", props), ("languages", "en")]);
            let json = self.api.get(HOST, &p).await?;
            let entities = json["entities"].as_object().into_iter().flat_map(Map::values);
            out.extend(entities.filter(|e| e.get("missing").is_none()).cloned());
        }
        Ok(out)
    }

    /// The current state of an item. Redirects are followed, so the returned
    /// id may differ from the requested one (merged items).
    pub async fn item(&self, id: ItemId) -> Result<Option<Entity>> {
        let mut entities = self.entities(&[id.to_string()], "claims").await?;
        Ok(entities.pop().and_then(Entity::new))
    }

    pub async fn property(&self, id: PropertyId) -> Result<Option<PropertyInfo>> {
        let mut entities = self.entities(&[id.to_string()], "claims|datatype|labels").await?;
        Ok(entities.pop().map(|json| PropertyInfo::from_json(id, &json)))
    }

    /// Labels, aliases and unit symbols (P5061) of items, in the given languages (`en|de`).
    pub async fn names(&self, ids: &[ItemId], languages: &str) -> Result<HashMap<ItemId, Vec<String>>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(IDS_PER_REQUEST) {
            let ids = chunk.iter().map(ItemId::to_string).collect::<Vec<_>>().join("|");
            let p = params(&[
                ("action", "wbgetentities"),
                ("ids", &ids),
                ("props", "labels|aliases|claims"),
                ("languages", languages),
            ]);
            let json = self.api.get(HOST, &p).await?;
            for entity in json["entities"].as_object().into_iter().flat_map(Map::values) {
                let Some(id) = entity["id"].as_str().and_then(|q| q.parse().ok()) else { continue };
                let labels = entity["labels"].as_object().into_iter().flat_map(Map::values);
                let aliases = entity["aliases"]
                    .as_object()
                    .into_iter()
                    .flat_map(Map::values)
                    .flat_map(|a| a.as_array().into_iter().flatten());
                let symbols = entity["claims"][UNIT_SYMBOL.to_string()]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| &c["mainsnak"]["datavalue"]["value"]);
                let names = labels
                    .chain(aliases)
                    .filter_map(|n| n["value"].as_str())
                    .chain(symbols.filter_map(|v| v["text"].as_str()));
                out.insert(id, names.map(str::to_string).collect());
            }
        }
        Ok(out)
    }

    /// Was `value` removed from `property` of this item before (#89)? Looks for
    /// `wbremoveclaims` in the latest 500 edit summaries. Unknown (`false`) for
    /// values whose summary form we cannot reproduce.
    pub async fn was_removed(&self, item: ItemId, property: PropertyId, value: &Value) -> Result<bool> {
        let Some(text) = value.summary_text() else { return Ok(false) };
        let id = item.to_string();
        let p = params(&[
            ("action", "query"),
            ("prop", "revisions"),
            ("titles", &id),
            ("rvprop", "comment"),
            ("rvlimit", "max"),
        ]);
        let json = self.api.get(HOST, &p).await?;
        let revisions = json["query"]["pages"][0]["revisions"].as_array().cloned().unwrap_or_default();
        let needle = format!("[[Property:{property}]]: {text}");
        Ok(revisions
            .iter()
            .filter_map(|r| r["comment"].as_str())
            .any(|c| c.contains("wbremoveclaims-remove") && mentions(c, &needle)))
    }

    /// A date via Wikibase's own parser (`wbparsevalue`), which knows MediaWiki's
    /// month names in every language (#56). `None` if it cannot parse the text,
    /// or only to decade or century precision.
    /// Best effort: an API error (Wikibase reports some unparsable text that way) is also `None`.
    pub async fn parse_time(&self, text: &str, lang: &str) -> Option<Date> {
        let options = serde_json::json!({ "lang": lang }).to_string();
        let p = params(&[("action", "wbparsevalue"), ("datatype", "time"), ("values", text), ("options", &options)]);
        let json = self.api.get(HOST, &p).await.inspect_err(|e| tracing::debug!("wbparsevalue: {e:#}")).ok()?;
        let value = &json["results"][0]["value"];
        let precise_enough = value["precision"].as_u64().is_some_and(|p| p >= 9);
        existing_date(value).filter(|d| precise_enough && d.year > 0)
    }

    /// English labels, falling back to the id.
    pub async fn labels(&self, ids: &[String]) -> Result<HashMap<String, String>> {
        let entities = self.entities(ids, "labels").await?;
        Ok(entities
            .iter()
            .filter_map(|e| {
                let id = e["id"].as_str()?.to_string();
                let label = e["labels"]["en"]["value"].as_str().unwrap_or(&id).to_string();
                Some((id, label))
            })
            .collect())
    }
}

/// `needle` occurs in `text`, followed by its end or a separator (so `tt1` does not match `tt12`).
fn mentions(text: &str, needle: &str) -> bool {
    text.match_indices(needle)
        .any(|(i, _)| text[i + needle.len()..].chars().next().is_none_or(|c| matches!(c, ',' | ' ' | ';')))
}

/// An item with its statements.
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: ItemId,
    claims: Map<String, Json>,
}

impl Entity {
    pub fn new(json: Json) -> Option<Self> {
        let id = json["id"].as_str()?.parse().ok()?;
        let claims = json.get("claims").and_then(Json::as_object).cloned().unwrap_or_default();
        Some(Self { id, claims })
    }

    pub fn statements(&self, property: PropertyId) -> &[Json] {
        self.claims.get(&property.to_string()).and_then(Json::as_array).map_or(&[], Vec::as_slice)
    }

    pub fn has_property(&self, property: PropertyId) -> bool {
        !self.statements(property).is_empty()
    }

    /// Does a non-deprecated statement already say this (or something more precise)?
    pub fn has_value(&self, property: PropertyId, value: &Value) -> bool {
        self.statements(property)
            .iter()
            .filter(|s| s["rank"] != "deprecated")
            .any(|s| value.matches(&s["mainsnak"]["datavalue"]))
    }

    /// Item values of `property`, e.g. the classes in P31.
    pub fn item_values(&self, property: PropertyId) -> Vec<ItemId> {
        self.statements(property)
            .iter()
            .filter_map(|s| s["mainsnak"]["datavalue"]["value"]["id"].as_str()?.parse().ok())
            .collect()
    }

    pub fn property_ids(&self) -> Vec<PropertyId> {
        self.claims.keys().filter_map(|k| k.parse().ok()).collect()
    }
}

/// Ordered from weakest to strictest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConstraintStatus {
    Suggestion,
    Normal,
    Mandatory,
}

/// One "property constraint" (P2302) statement of a property.
#[derive(Debug, Clone, Serialize)]
pub struct ConstraintDef {
    /// The constraint type, e.g. Q21502410 (distinct values).
    pub kind: ItemId,
    pub status: ConstraintStatus,
    #[serde(skip)]
    qualifiers: Map<String, Json>,
}

impl ConstraintDef {
    pub(crate) fn from_statement(statement: &Json) -> Option<Self> {
        let kind = statement["mainsnak"]["datavalue"]["value"]["id"].as_str()?.parse().ok()?;
        let qualifiers = statement.get("qualifiers").and_then(Json::as_object).cloned().unwrap_or_default();
        let mut def = Self { kind, status: ConstraintStatus::Normal, qualifiers };
        def.status = match def.items(CONSTRAINT_STATUS).first() {
            Some(&MANDATORY) => ConstraintStatus::Mandatory,
            Some(&SUGGESTION) => ConstraintStatus::Suggestion,
            _ => ConstraintStatus::Normal,
        };
        Some(def)
    }

    /// Qualifier snaks for `property`.
    pub fn snaks(&self, property: PropertyId) -> &[Json] {
        self.qualifiers.get(&property.to_string()).and_then(Json::as_array).map_or(&[], Vec::as_slice)
    }

    /// Entity-id qualifier values (item or property ids, as strings).
    pub fn entity_ids(&self, property: PropertyId) -> Vec<String> {
        let snaks = self.snaks(property).iter();
        snaks.filter_map(|s| s["datavalue"]["value"]["id"].as_str().map(str::to_string)).collect()
    }

    pub fn items(&self, property: PropertyId) -> Vec<ItemId> {
        self.entity_ids(property).iter().filter_map(|id| id.parse().ok()).collect()
    }

    pub fn first_property(&self, property: PropertyId) -> Option<PropertyId> {
        self.entity_ids(property).iter().find_map(|id| id.parse().ok())
    }

    pub fn first_string(&self, property: PropertyId) -> Option<&str> {
        self.snaks(property).iter().find_map(|s| s["datavalue"]["value"].as_str())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PropertyInfo {
    pub id: PropertyId,
    pub label: String,
    /// The raw Wikibase datatype name, also when it is not supported.
    pub datatype_name: String,
    pub datatype: Option<Datatype>,
    pub deprecated: bool,
    pub constraints: Vec<ConstraintDef>,
    /// P1630, with `$1` for the value: turns external ids into links.
    pub formatter_url: Option<String>,
}

impl PropertyInfo {
    fn from_json(id: PropertyId, json: &Json) -> Self {
        let entity = Entity::new(json.clone());
        let deprecated = entity
            .as_ref()
            .is_some_and(|e| e.item_values(INSTANCE_OF).iter().any(|c| DEPRECATED_PROPERTY_CLASSES.contains(c)));
        let statements = json["claims"][PROPERTY_CONSTRAINT.to_string()].as_array();
        let datatype_name = json["datatype"].as_str().unwrap_or_default().to_string();
        Self {
            id,
            label: json["labels"]["en"]["value"].as_str().map_or_else(|| id.to_string(), str::to_string),
            datatype: Datatype::from_wikibase(&datatype_name),
            datatype_name,
            deprecated,
            formatter_url: json["claims"][FORMATTER_URL.to_string()]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["rank"] != "deprecated")
                .find_map(|s| s["mainsnak"]["datavalue"]["value"].as_str().map(str::to_string)),
            constraints: statements.into_iter().flatten().filter_map(ConstraintDef::from_statement).collect(),
        }
    }

    pub fn constraint(&self, kind: ItemId) -> Option<&ConstraintDef> {
        self.constraints.iter().find(|c| c.kind == kind)
    }

    /// Units allowed by the "allowed units" constraint (`None` = no unit),
    /// or `None` if the property does not restrict units.
    pub fn allowed_units(&self) -> Option<Vec<Option<ItemId>>> {
        let snaks = self.constraint(ALLOWED_UNITS)?.snaks(ITEM_OF_CONSTRAINT).iter();
        Some(snaks.map(|s| s["datavalue"]["value"]["id"].as_str().and_then(|id| id.parse().ok())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item_snak(id: &str) -> Json {
        json!({"snaktype": "value", "datavalue": {"type": "wikibase-entityid", "value": {"id": id}}})
    }

    #[test]
    fn property_info() {
        let json = json!({
            "id": "P345", "datatype": "external-id", "labels": {"en": {"value": "IMDb ID"}},
            "claims": {"P2302": [
                {"mainsnak": item_snak("Q21502410"), "qualifiers": {"P2316": [item_snak("Q21502408")]}},
                {"mainsnak": item_snak("Q21502404"), "qualifiers": {"P1793": [
                    {"snaktype": "value", "datavalue": {"type": "string", "value": "tt\\d+"}}
                ]}},
                {"mainsnak": item_snak("Q19474404"), "qualifiers": {"P2316": [item_snak("Q62026391")]}}
            ]}
        });
        let info = PropertyInfo::from_json(PropertyId(345), &json);
        assert_eq!(info.label, "IMDb ID");
        assert_eq!(info.datatype, Some(Datatype::ExternalId));
        assert!(!info.deprecated);
        let statuses: Vec<_> = info.constraints.iter().map(|c| c.status).collect();
        assert_eq!(statuses, [ConstraintStatus::Mandatory, ConstraintStatus::Normal, ConstraintStatus::Suggestion]);
        assert_eq!(info.constraint(ItemId(21_502_404)).unwrap().first_string(PropertyId(1793)), Some("tt\\d+"));
    }

    #[test]
    fn removal_summaries() {
        let removal = "/* wbremoveclaims-remove:1| */ [[Property:P345]]: tt12";
        assert!(mentions(removal, "[[Property:P345]]: tt12"));
        assert!(!mentions(removal, "[[Property:P345]]: tt1"), "a prefix of another value");
        assert!(mentions(
            "/* wbremoveclaims-remove:1| */ [[Property:P19]]: [[Q90]], wrong place",
            "[[Property:P19]]: [[Q90]]"
        ));
        let date = |year, month, day| Value::Time {
            date: crate::value::Date { year, month, day },
            calendar: crate::value::Calendar::Gregorian,
        };
        assert_eq!(date(1928, 2, 2).summary_text().as_deref(), Some("2 February 1928"));
        assert_eq!(date(1928, 2, 0).summary_text().as_deref(), Some("February 1928"));
        assert_eq!(date(1928, 0, 0).summary_text().as_deref(), Some("1928"));
        assert_eq!(Value::Item(ItemId(5)).summary_text().as_deref(), Some("[[Q5]]"));
    }

    #[tokio::test]
    #[ignore = "requires database / external services — run with cargo test -- --ignored"]
    async fn live_removal_is_found() {
        let http =
            crate::app_state::http_client("HarvestTemplates tests (https://harvesttemplates.toolforge.org)").unwrap();
        let wikidata = Wikidata { api: MwApi::new(http) };
        let isni = |s: &str| Value::String(s.to_string());
        assert!(wikidata.was_removed(ItemId(5_716_580), PropertyId(213), &isni("0000000116716546")).await.unwrap());
        assert!(!wikidata.was_removed(ItemId(5_716_580), PropertyId(213), &isni("0000000000000000")).await.unwrap());
    }

    #[test]
    fn existing_values() {
        let entity = Entity::new(json!({"id": "Q1", "claims": {"P31": [
            {"rank": "normal", "mainsnak": item_snak("Q5")},
            {"rank": "deprecated", "mainsnak": item_snak("Q6")}
        ]}}))
        .unwrap();
        assert!(entity.has_property(PropertyId(31)));
        assert!(!entity.has_property(PropertyId(21)));
        assert!(entity.has_value(PropertyId(31), &Value::Item(ItemId(5))));
        assert!(!entity.has_value(PropertyId(31), &Value::Item(ItemId(6))), "deprecated statements don't count");
    }
}
