//! Turning a cleaned template value into a Wikibase data value.
//! Network-dependent steps (item lookup, file existence) live in the harvest pipeline.

mod coordinate;
mod links;
mod numerals;
mod quantity;
mod time;
mod transform;

pub use coordinate::{Coordinate, parse_coordinate, parse_coordinate_parts};
pub use links::{LinkChoice, file_name, link_target, url};
pub use quantity::{DecimalMark, parse_amount, split_unit, unit_key};
pub use time::{Calendar, Date, DateLimit, Relation, parse_date, parse_date_parts};
pub use transform::{Transform, TransformSpec};

use crate::ids::ItemId;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

/// Property datatypes HarvestTemplates can write, named as in the Wikibase API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Datatype {
    #[serde(rename = "wikibase-item")]
    Item,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "external-id")]
    ExternalId,
    #[serde(rename = "url")]
    Url,
    #[serde(rename = "commonsMedia")]
    CommonsMedia,
    #[serde(rename = "time")]
    Time,
    #[serde(rename = "quantity")]
    Quantity,
    #[serde(rename = "monolingualtext")]
    Monolingual,
    #[serde(rename = "globe-coordinate")]
    GlobeCoordinate,
}

impl Datatype {
    /// `None` for datatypes we cannot harvest (yet), e.g. `globe-coordinate`.
    pub fn from_wikibase(name: &str) -> Option<Self> {
        serde_json::from_value(Json::String(name.to_string())).ok()
    }

    pub fn wikibase_name(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
    }
}

/// Why a value could not be used. The message is shown to the user per row.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("could not find a date")]
    NoDate,
    #[error("imprecise date")]
    ImpreciseDate,
    #[error("ambiguous date: several years")]
    AmbiguousDate,
    #[error("invalid date")]
    InvalidDate,
    #[error("date outside the configured range")]
    OutsideDateLimit,
    #[error("unclear number")]
    UnclearNumber,
    #[error("no link to a target page")]
    NoLink,
    #[error("link to a section, not a page")]
    SectionLink,
    #[error("not a file name")]
    NotAFile,
    #[error("not a URL")]
    NotAUrl,
    #[error("could not find a coordinate")]
    NoCoordinate,
}

/// A parsed value, ready to become a Wikibase snak.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Value {
    Item(ItemId),
    /// `string`, `external-id`, `url` and `commonsMedia`.
    String(String),
    Time {
        date: Date,
        calendar: Calendar,
    },
    Quantity {
        amount: String,
        unit: Option<ItemId>,
    },
    Monolingual {
        text: String,
        language: String,
    },
    Coordinate(Coordinate),
}

impl Value {
    /// The `datavalue` object of a Wikibase snak.
    pub fn datavalue(&self) -> Json {
        match self {
            Self::Item(q) => json!({
                "type": "wikibase-entityid",
                "value": { "entity-type": "item", "numeric-id": q.0, "id": q.to_string() },
            }),
            Self::String(s) => json!({ "type": "string", "value": s }),
            Self::Time { date, calendar } => json!({
                "type": "time",
                "value": {
                    "time": date.wikibase_time(),
                    "timezone": 0, "before": 0, "after": 0,
                    "precision": date.precision(),
                    "calendarmodel": entity_uri(calendar.item()),
                },
            }),
            Self::Quantity { amount, unit } => json!({
                "type": "quantity",
                "value": { "amount": amount, "unit": unit.map_or_else(|| "1".to_string(), entity_uri) },
            }),
            Self::Monolingual { text, language } => json!({
                "type": "monolingualtext",
                "value": { "text": text, "language": language },
            }),
            Self::Coordinate(c) => json!({
                "type": "globecoordinate",
                "value": {
                    "latitude": c.latitude, "longitude": c.longitude, "altitude": null,
                    "precision": c.precision, "globe": entity_uri(EARTH),
                },
            }),
        }
    }

    /// Does an existing Wikibase `datavalue` already express this value?
    /// An existing date counts if it is at least as precise and agrees on our fields.
    pub fn matches(&self, datavalue: &Json) -> bool {
        let v = &datavalue["value"];
        match self {
            Self::Item(q) => v["id"].as_str() == Some(q.to_string().as_str()),
            Self::String(s) => v.as_str() == Some(s.as_str()),
            Self::Time { date, .. } => existing_date(v).is_some_and(|d| {
                d.year == date.year
                    && (date.month == 0 || d.month == date.month)
                    && (date.day == 0 || d.day == date.day)
            }),
            Self::Quantity { amount, .. } => {
                let parse = |s: &str| s.parse::<f64>().ok();
                matches!((v["amount"].as_str().and_then(parse), parse(amount)), (Some(a), Some(b)) if (a - b).abs() < f64::EPSILON)
            }
            Self::Monolingual { text, language } => v["text"] == text.as_str() && v["language"] == language.as_str(),
            Self::Coordinate(c) => {
                let tolerance = c.precision.max(0.001);
                let near =
                    |key: &str, ours: f64| v[key].as_f64().is_some_and(|theirs| (theirs - ours).abs() <= tolerance);
                near("latitude", c.latitude) && near("longitude", c.longitude)
            }
        }
    }

    /// Short human-readable form for the results table.
    pub fn display(&self) -> String {
        match self {
            Self::Item(q) => q.to_string(),
            Self::String(s) => s.clone(),
            Self::Time { date, calendar: Calendar::Julian } => format!("{date} (Julian)"),
            Self::Time { date, .. } => date.to_string(),
            Self::Quantity { amount, unit: Some(u) } => format!("{amount} {u}"),
            Self::Quantity { amount, unit: None } => amount.clone(),
            Self::Monolingual { text, language } => format!("{text} ({language})"),
            Self::Coordinate(c) => {
                let decimals = (-c.precision.log10()).ceil().clamp(0.0, 9.0) as usize;
                format!("{:.decimals$}, {:.decimals$}", c.latitude, c.longitude)
            }
        }
    }
}

const EARTH: ItemId = ItemId(2);

/// Year, month and day of a Wikibase time value, honouring its precision.
pub fn existing_date(value: &Json) -> Option<Date> {
    let time = value["time"].as_str()?.strip_prefix('+')?;
    let mut parts = time.split(['-', 'T']);
    let year = parts.next()?.parse().ok()?;
    let (month, day): (u8, u8) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    let precision = value["precision"].as_u64().unwrap_or(11);
    Some(Date { year, month: if precision >= 10 { month } else { 0 }, day: if precision >= 11 { day } else { 0 } })
}

/// The value as a SPARQL term, for the datatypes WDQS checks need.
pub fn sparql_term(value: &Value, datatype: Datatype) -> Option<String> {
    match (value, datatype) {
        (Value::Item(q), _) => Some(format!("wd:{q}")),
        (Value::String(s), Datatype::CommonsMedia) => Some(format!(
            "<http://commons.wikimedia.org/wiki/Special:FilePath/{}>",
            urlencoding::encode(&s.replace(' ', "_"))
        )),
        (Value::String(s), _) => serde_json::to_string(s).ok(),
        _ => None,
    }
}

pub fn entity_uri(id: impl std::fmt::Display) -> String {
    format!("http://www.wikidata.org/entity/{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datatype_names_round_trip() {
        for name in [
            "wikibase-item",
            "string",
            "external-id",
            "url",
            "commonsMedia",
            "time",
            "quantity",
            "monolingualtext",
            "globe-coordinate",
        ] {
            assert_eq!(Datatype::from_wikibase(name).unwrap().wikibase_name(), name);
        }
        assert_eq!(Datatype::from_wikibase("geo-shape"), None);
    }

    #[test]
    fn datavalues() {
        let v = Value::Quantity { amount: "+5".into(), unit: None };
        assert_eq!(v.datavalue()["value"]["unit"], "1");
        let v = Value::Time { date: Date { year: 1950, month: 5, day: 0 }, calendar: Calendar::Julian };
        assert_eq!(v.datavalue()["value"]["precision"], 10);
        assert_eq!(v.datavalue()["value"]["calendarmodel"], "http://www.wikidata.org/entity/Q1985786");
        assert_eq!(Value::Item(ItemId(42)).datavalue()["value"]["id"], "Q42");
    }

    #[test]
    fn matching_existing_values() {
        let time = |time: &str, precision: u8| json!({"value": {"time": time, "precision": precision}});
        let year = Value::Time { date: Date { year: 1950, month: 0, day: 0 }, calendar: Calendar::Gregorian };
        let day = Value::Time { date: Date { year: 1950, month: 5, day: 12 }, calendar: Calendar::Gregorian };
        assert!(year.matches(&time("+1950-05-12T00:00:00Z", 11)), "a more precise date covers a year");
        assert!(!day.matches(&time("+1950-00-00T00:00:00Z", 9)), "a year does not cover a full date");
        assert!(day.matches(&time("+1950-05-12T00:00:00Z", 11)));
        let qty = Value::Quantity { amount: "+62".into(), unit: None };
        assert!(qty.matches(&json!({"value": {"amount": "+62.0"}})));
        assert!(Value::String("tt1".into()).matches(&json!({"value": "tt1"})));
        assert!(!Value::String("tt1".into()).matches(&json!({"value": "tt2"})));
    }
}
