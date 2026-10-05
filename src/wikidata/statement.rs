use crate::ids::{ItemId, PropertyId};
use crate::value::{Datatype, Value};
use serde_json::{Value as Json, json};

const IMPORTED_FROM: PropertyId = PropertyId(143);
const WIKIMEDIA_IMPORT_URL: PropertyId = PropertyId(4656);

/// Where a value came from: the wiki's item and the exact page revision.
#[derive(Debug, Clone)]
pub struct Source {
    pub wiki: Option<ItemId>,
    pub permalink: String,
}

impl Source {
    pub fn new(wiki: Option<ItemId>, host: &str, title: &str, revision: u64) -> Self {
        let title = urlencoding::encode(&title.replace(' ', "_")).into_owned();
        Self {
            wiki,
            permalink: format!("https://{host}/w/index.php?title={title}&oldid={revision}"),
        }
    }

    fn reference(&self) -> Json {
        let mut snaks = serde_json::Map::new();
        let mut order = vec![];
        if let Some(wiki) = self.wiki {
            snaks.insert(
                IMPORTED_FROM.to_string(),
                json!([snak(IMPORTED_FROM, Datatype::Item, &Value::Item(wiki))]),
            );
            order.push(IMPORTED_FROM.to_string());
        }
        let url = Value::String(self.permalink.clone());
        snaks.insert(
            WIKIMEDIA_IMPORT_URL.to_string(),
            json!([snak(WIKIMEDIA_IMPORT_URL, Datatype::Url, &url)]),
        );
        order.push(WIKIMEDIA_IMPORT_URL.to_string());
        json!({ "snaks": snaks, "snaks-order": order })
    }
}

pub fn snak(property: PropertyId, datatype: Datatype, value: &Value) -> Json {
    json!({
        "snaktype": "value",
        "property": property.to_string(),
        "datatype": datatype.wikibase_name(),
        "datavalue": value.datavalue(),
    })
}

/// A new statement, as used in `wbeditentity` `data.claims`.
pub fn statement(property: PropertyId, datatype: Datatype, value: &Value, source: &Source) -> Json {
    json!({
        "type": "statement",
        "rank": "normal",
        "mainsnak": snak(property, datatype, value),
        "references": [source.reference()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_with_reference() {
        let source = Source::new(Some(ItemId(328)), "en.wikipedia.org", "The Shawshank Redemption", 123);
        let s = statement(
            PropertyId(345),
            Datatype::ExternalId,
            &Value::String("tt0111161".into()),
            &source,
        );
        assert_eq!(s["mainsnak"]["property"], "P345");
        assert_eq!(s["mainsnak"]["datatype"], "external-id");
        assert_eq!(s["mainsnak"]["datavalue"]["value"], "tt0111161");
        let reference = &s["references"][0];
        assert_eq!(reference["snaks"]["P143"][0]["datavalue"]["value"]["id"], "Q328");
        assert_eq!(
            reference["snaks"]["P4656"][0]["datavalue"]["value"],
            "https://en.wikipedia.org/w/index.php?title=The_Shawshank_Redemption&oldid=123"
        );
        assert_eq!(reference["snaks-order"], json!(["P143", "P4656"]));
    }
}
