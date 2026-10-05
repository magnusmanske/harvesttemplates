//! Checks that need other entities: asked of WDQS, Commons or Wikidata search.

use super::local::{ITEM_OF_CONSTRAINT, PROPERTY, has_statement};
use super::{Candidate, Services};
use crate::ids::{ItemId, PropertyId};
use crate::value::{Value, sparql_term};
use crate::wiki::api::params;
use crate::wikidata::{ConstraintDef, HOST};
use anyhow::Result;

const CLASS: PropertyId = PropertyId(2308);
const RELATION: PropertyId = PropertyId(2309);
const NAMESPACE: PropertyId = PropertyId(2307);
const INSTANCE_OF: PropertyId = PropertyId(31);
const RELATION_SUBCLASS: ItemId = ItemId(21_514_624);
const RELATION_EITHER: ItemId = ItemId(30_208_840);
const COMMONS: &str = "commons.wikimedia.org";

#[derive(Debug, Clone, Copy)]
pub enum Remote {
    CommonsLink,
    ConflictsWith,
    Distinct,
    Inverse,
    Symmetric,
    Type,
    ValueRequiresStatement,
    ValueType,
}

impl Remote {
    /// `true` on violation.
    pub async fn violated(self, def: &ConstraintDef, c: &Candidate<'_>, s: Services<'_>) -> Result<bool> {
        let q = c.item.id;
        let value_item = match c.value {
            Value::Item(v) => Some(*v),
            _ => None,
        };
        match (self, value_item) {
            (Self::CommonsLink, _) => commons_link(def, c, s).await,
            (Self::ConflictsWith, _) => conflicts_with(def, c, s).await,
            (Self::Distinct, _) => distinct(c, s).await,
            (Self::Inverse, Some(v)) => match def.first_property(PROPERTY) {
                Some(inverse) => Ok(!s.wdqs.ask(&format!("ASK {{ wd:{v} wdt:{inverse} wd:{q} }}")).await?),
                None => Ok(false),
            },
            (Self::Symmetric, Some(v)) => Ok(!s
                .wdqs
                .ask(&format!("ASK {{ wd:{v} wdt:{} wd:{q} }}", c.property))
                .await?),
            (Self::Type, _) => {
                let direct = c.item.item_values(INSTANCE_OF);
                if relation(def) != "wdt:P279" && def.items(CLASS).iter().any(|class| direct.contains(class)) {
                    return Ok(false);
                }
                in_class(&format!("wd:{q}"), def, s).await.map(|ok| !ok)
            }
            (Self::ValueType, Some(v)) => in_class(&format!("wd:{v}"), def, s).await.map(|ok| !ok),
            (Self::ValueRequiresStatement, Some(v)) => match def.first_property(PROPERTY) {
                Some(required) => {
                    let sparql = format!("ASK {{ wd:{v} wdt:{required} {} }}", object_filter(def));
                    Ok(!s.wdqs.ask(&sparql).await?)
                }
                None => Ok(false),
            },
            _ => Ok(false),
        }
    }
}

/// `[]` (anything), or a variable restricted to the constraint's item list.
fn object_filter(def: &ConstraintDef) -> String {
    let values = def.items(ITEM_OF_CONSTRAINT);
    if values.is_empty() {
        return "[]".to_string();
    }
    let list: Vec<String> = values.iter().map(|v| format!("wd:{v}")).collect();
    format!("?v . VALUES ?v {{ {} }}", list.join(" "))
}

fn relation(def: &ConstraintDef) -> &'static str {
    match def.items(RELATION).first() {
        Some(&RELATION_SUBCLASS) => "wdt:P279",
        Some(&RELATION_EITHER) => "(wdt:P31|wdt:P279)",
        _ => "wdt:P31",
    }
}

async fn in_class(subject: &str, def: &ConstraintDef, s: Services<'_>) -> Result<bool> {
    let classes: Vec<String> = def.items(CLASS).iter().map(|c| format!("wd:{c}")).collect();
    if classes.is_empty() {
        return Ok(true);
    }
    let sparql = format!(
        "ASK {{ VALUES ?class {{ {} }} {subject} {}/wdt:P279* ?class }}",
        classes.join(" "),
        relation(def)
    );
    s.wdqs.ask(&sparql).await
}

/// The file or page must exist on Commons, in the namespace the constraint names (#23).
async fn commons_link(def: &ConstraintDef, c: &Candidate<'_>, s: Services<'_>) -> Result<bool> {
    let Value::String(name) = c.value else {
        return Ok(false);
    };
    let namespace = def.first_string(NAMESPACE).unwrap_or_default();
    let title = if namespace.is_empty() {
        name.clone()
    } else {
        format!("{namespace}:{name}")
    };
    let p = params(&[("action", "query"), ("titles", &title), ("redirects", "1")]);
    let json = s.mw.get(COMMONS, &p).await?;
    let page = &json["query"]["pages"][0];
    let exists = page.get("missing").is_none() && page.get("invalid").is_none();
    let right_namespace = !namespace.is_empty() || page["ns"] == 0;
    Ok(!(exists && right_namespace))
}

/// Our property says it conflicts with another statement on the item, or a
/// property on the item says it conflicts with ours.
async fn conflicts_with(def: &ConstraintDef, c: &Candidate<'_>, s: Services<'_>) -> Result<bool> {
    let ours = def.first_property(PROPERTY);
    if ours.is_some_and(|other| has_statement(c, other, &def.items(ITEM_OF_CONSTRAINT))) {
        return Ok(true);
    }
    let value_filter = match sparql_term(c.value, c.datatype) {
        Some(term) => format!("FILTER(!BOUND(?v) || ?v = {term})"),
        None => "FILTER(!BOUND(?v))".to_string(),
    };
    let sparql = format!(
        "ASK {{ wd:{} ?claim [] . ?prop wikibase:claim ?claim ; p:P2302 ?c . \
         ?c ps:P2302 wd:Q21502838 ; pq:P2306 wd:{} . OPTIONAL {{ ?c pq:P2305 ?v }} {value_filter} }}",
        c.item.id, c.property
    );
    s.wdqs.ask(&sparql).await
}

/// Does another item already have this value? Uses search (`haswbstatement`),
/// which unlike WDQS covers all items; WDQS only for values search can't express.
async fn distinct(c: &Candidate<'_>, s: Services<'_>) -> Result<bool> {
    let key = match c.value {
        Value::Item(q) => Some(q.to_string()),
        Value::String(v) if !v.contains(|ch: char| ch.is_whitespace() || ch == '"') => Some(v.clone()),
        _ => None,
    };
    let Some(key) = key else {
        let Some(term) = sparql_term(c.value, c.datatype) else {
            return Ok(false);
        };
        let p = c.property;
        return s
            .wdqs
            .ask(&format!(
                "ASK {{ ?item p:{p}/ps:{p} {term} . FILTER(?item != wd:{}) }}",
                c.item.id
            ))
            .await;
    };
    let query = format!("haswbstatement:{}={key}", c.property);
    let p = params(&[
        ("action", "query"),
        ("list", "search"),
        ("srsearch", &query),
        ("srlimit", "5"),
        ("srprop", ""),
    ]);
    let json = s.mw.get(HOST, &p).await?;
    let own = c.item.id.to_string();
    let hits = json["query"]["search"].as_array().into_iter().flatten();
    Ok(hits.filter_map(|h| h["title"].as_str()).any(|t| t != own))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Datatype;
    use crate::wiki::MwApi;
    use crate::wikidata::{Entity, Wdqs};
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn def(qualifiers: serde_json::Value) -> ConstraintDef {
        ConstraintDef::from_statement(
            &json!({"mainsnak": {"datavalue": {"value": {"id": "Q1"}}}, "qualifiers": qualifiers}),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn distinct_uses_search_and_ignores_own_item() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("haswbstatement%3AP345%3Dtt1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"query": {"search": [{"title": "Q1"}]}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("haswbstatement%3AP345%3Dtt2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"query": {"search": [{"title": "Q9"}]}})))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let (mw, wdqs) = (
            MwApi::with_base_url(http.clone(), server.uri()),
            Wdqs::new(http, &server.uri()),
        );
        let s = Services { wdqs: &wdqs, mw: &mw };
        let item = Entity::new(json!({"id": "Q1", "claims": {}})).unwrap();
        for (value, expected) in [("tt1", false), ("tt2", true)] {
            let value = Value::String(value.into());
            let c = Candidate {
                item: &item,
                property: PropertyId(345),
                datatype: Datatype::ExternalId,
                value: &value,
                qualifiers: &[],
            };
            assert_eq!(
                Remote::Distinct.violated(&def(json!({})), &c, s).await.unwrap(),
                expected
            );
        }
    }

    #[test]
    fn sparql_fragments() {
        let d = def(json!({"P2305": [{"snaktype": "value", "datavalue": {"value": {"id": "Q5"}}}]}));
        assert_eq!(object_filter(&d), "?v . VALUES ?v { wd:Q5 }");
        assert_eq!(object_filter(&def(json!({}))), "[]");
        let either = def(json!({"P2309": [{"snaktype": "value", "datavalue": {"value": {"id": "Q30208840"}}}]}));
        assert_eq!(relation(&either), "(wdt:P31|wdt:P279)");
    }

    #[tokio::test]
    #[ignore = "requires database / external services — run with cargo test -- --ignored"]
    async fn live_sparql_is_valid() {
        let http =
            crate::app_state::http_client("HarvestTemplates tests (https://github.com/magnusmanske/harvesttemplates)")
                .unwrap();
        let (mw, wdqs) = (
            MwApi::new(http.clone()),
            Wdqs::new(http, crate::wikidata::wdqs::ENDPOINT),
        );
        let s = Services { wdqs: &wdqs, mw: &mw };
        let item = |c| Entity::new(json!({"id": "Q42", "claims": c})).unwrap();
        let human = def(json!({"P2308": [{"snaktype": "value", "datavalue": {"value": {"id": "Q5"}}}]}));
        let film = def(json!({"P2308": [{"snaktype": "value", "datavalue": {"value": {"id": "Q11424"}}}]}));
        let value = Value::String("nm0010930".into());
        let no_p31 = item(json!({}));
        let c = |i| Candidate {
            item: i,
            property: PropertyId(345),
            datatype: Datatype::ExternalId,
            value: &value,
            qualifiers: &[],
        };
        assert!(
            !Remote::Type.violated(&human, &c(&no_p31), s).await.unwrap(),
            "Q42 is a human (via WDQS)"
        );
        assert!(Remote::Type.violated(&film, &c(&no_p31), s).await.unwrap());
        assert!(
            !Remote::ConflictsWith
                .violated(&def(json!({})), &c(&no_p31), s)
                .await
                .unwrap()
        );
        assert!(
            !Remote::Distinct
                .violated(&def(json!({})), &c(&no_p31), s)
                .await
                .unwrap(),
            "only Q42 has it"
        );
        let other = Value::String("nm0000001".into());
        let c2 = Candidate {
            item: &no_p31,
            property: PropertyId(345),
            datatype: Datatype::ExternalId,
            value: &other,
            qualifiers: &[],
        };
        assert!(
            Remote::Distinct.violated(&def(json!({})), &c2, s).await.unwrap(),
            "Fred Astaire has nm0000001"
        );
        let file = Value::String("Douglas adams portrait cropped.jpg".into());
        let c3 = Candidate {
            item: &no_p31,
            property: PropertyId(18),
            datatype: Datatype::CommonsMedia,
            value: &file,
            qualifiers: &[],
        };
        let in_file_ns = def(json!({"P2307": [{"snaktype": "value", "datavalue": {"value": "File"}}]}));
        assert!(!Remote::CommonsLink.violated(&in_file_ns, &c3, s).await.unwrap());
    }
}
