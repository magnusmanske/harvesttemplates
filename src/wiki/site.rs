use super::api::{MwApi, params};
use crate::ids::ItemId;
use crate::wikitext::{TemplateMatcher, uppercase_first};
use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Projects selectable in the form, as in the original tool.
pub const PROJECTS: [&str; 10] = [
    "wikipedia",
    "wikibooks",
    "wikinews",
    "wikiquote",
    "wikisource",
    "wikiversity",
    "wikivoyage",
    "wiktionary",
    "commons",
    "species",
];

pub const NS_FILE: i32 = 6;
pub const NS_TEMPLATE: i32 = 10;
pub const NS_CATEGORY: i32 = 14;

/// Host name for the original tool's `siteid` + `project` pair. This is the
/// only way a host is built, so outbound requests can only reach Wikimedia wikis.
pub fn host_for(siteid: &str, project: &str) -> Result<String> {
    if !PROJECTS.contains(&project) {
        bail!("unknown project '{project}'");
    }
    match project {
        "commons" | "species" => Ok(format!("{project}.wikimedia.org")),
        _ if is_language_code(siteid) => Ok(format!("{siteid}.{project}.org")),
        _ => bail!("invalid wiki language code '{siteid}'"),
    }
}

fn is_language_code(s: &str) -> bool {
    (2..=20).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// What we need to know about a wiki, from `meta=siteinfo`.
#[derive(Debug, Clone, Serialize)]
pub struct Site {
    pub host: String,
    /// Database name, e.g. `enwiki`; also the Wikidata site id.
    pub dbname: String,
    /// Content language, selects month names for dates.
    pub lang: String,
    pub namespaces: BTreeMap<i32, String>,
    pub template_prefixes: Vec<String>,
    pub file_prefixes: Vec<String>,
    pub category_prefixes: Vec<String>,
    pub template_case_sensitive: bool,
    /// The Wikidata item for this wiki (via P1800), used in references.
    pub edition: Option<ItemId>,
}

impl Site {
    pub async fn load(api: &MwApi, host: &str) -> Result<Self> {
        let p = params(&[("action", "query"), ("meta", "siteinfo"), ("siprop", "general|namespaces|namespacealiases")]);
        Self::from_siteinfo(host, &api.get(host, &p).await?)
    }

    fn from_siteinfo(host: &str, json: &Value) -> Result<Self> {
        let query = &json["query"];
        let general = &query["general"];
        let dbname = general["wikiid"].as_str().unwrap_or_default().to_string();
        if dbname.is_empty() || !dbname.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(anyhow!("{host} reports an invalid dbname '{dbname}'"));
        }
        let namespaces: BTreeMap<i32, String> = query["namespaces"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, ns)| Some((ns["id"].as_i64()? as i32, ns["name"].as_str()?.to_string())))
            .collect();
        let prefixes = |id: i32| -> Vec<String> {
            let ns = &query["namespaces"][id.to_string()];
            let aliases = query["namespacealiases"].as_array().into_iter().flatten();
            [&ns["name"], &ns["canonical"]]
                .into_iter()
                .chain(aliases.filter(|a| a["id"] == id).map(|a| &a["alias"]))
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        Ok(Self {
            host: host.to_string(),
            lang: general["lang"].as_str().unwrap_or("en").to_string(),
            template_case_sensitive: query["namespaces"]["10"]["case"] == "case-sensitive",
            template_prefixes: prefixes(NS_TEMPLATE),
            file_prefixes: prefixes(NS_FILE),
            category_prefixes: prefixes(NS_CATEGORY),
            namespaces,
            dbname,
            edition: None,
        })
    }

    /// `Category:Foo bar` for namespace 14 and `Foo_bar`.
    pub fn full_title(&self, namespace: i32, title: &str) -> String {
        let title = title.replace('_', " ");
        match self.namespaces.get(&namespace) {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}:{title}"),
            _ => title,
        }
    }

    /// Database key of a template or category name typed by a user:
    /// `Kategorie:Deutsche person` → `Deutsche_person` (first letter upper-cased).
    pub fn db_key(&self, namespace: i32, title: &str) -> String {
        let prefixes = match namespace {
            NS_TEMPLATE => &self.template_prefixes,
            NS_CATEGORY => &self.category_prefixes,
            NS_FILE => &self.file_prefixes,
            _ => &Vec::new(),
        };
        let title = title.trim().trim_start_matches(':');
        let title = match title.split_once(':') {
            Some((p, rest)) if prefixes.iter().any(|x| x.eq_ignore_ascii_case(p.trim())) => rest,
            _ => title,
        };
        let key = title
            .split(|c: char| c == '_' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_");
        if namespace == NS_TEMPLATE && self.template_case_sensitive { key } else { uppercase_first(&key) }
    }

    /// Redirects to a template (names without prefix), or `None` if it does not exist.
    pub async fn template_redirects(&self, api: &MwApi, key: &str) -> Result<Option<Vec<String>>> {
        let title = self.full_title(NS_TEMPLATE, key);
        let p = params(&[
            ("action", "query"),
            ("titles", &title),
            ("prop", "redirects"),
            ("rdnamespace", "10"),
            ("rdlimit", "max"),
            ("rdprop", "title"),
        ]);
        let (mut exists, mut names) = (true, vec![]);
        api.query_continue(&self.host, &p, |json| {
            let page = &json["query"]["pages"][0];
            exists &= page.get("missing").is_none() && page.get("invalid").is_none();
            let redirects = page["redirects"].as_array().into_iter().flatten();
            names.extend(
                redirects.filter_map(|r| r["title"].as_str()).map(|t| self.db_key(NS_TEMPLATE, t).replace('_', " ")),
            );
            true
        })
        .await?;
        Ok(exists.then_some(names))
    }

    pub fn template_matcher(&self, names: impl IntoIterator<Item = impl AsRef<str>>) -> TemplateMatcher {
        TemplateMatcher::new(names, &self.template_prefixes, !self.template_case_sensitive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hosts() {
        assert_eq!(host_for("en", "wikipedia").unwrap(), "en.wikipedia.org");
        assert_eq!(host_for("zh-min-nan", "wikipedia").unwrap(), "zh-min-nan.wikipedia.org");
        assert_eq!(host_for("whatever", "commons").unwrap(), "commons.wikimedia.org");
        assert!(host_for("evil.com/", "wikipedia").is_err());
        assert!(host_for("en", "example").is_err());
        assert!(host_for("EN", "wikipedia").is_err());
    }

    fn siteinfo() -> Value {
        json!({"query": {
            "general": {"wikiid": "dewiki", "lang": "de"},
            "namespaces": {
                "0": {"id": 0, "name": "", "case": "first-letter"},
                "6": {"id": 6, "name": "Datei", "canonical": "File", "case": "first-letter"},
                "10": {"id": 10, "name": "Vorlage", "canonical": "Template", "case": "first-letter"},
                "14": {"id": 14, "name": "Kategorie", "canonical": "Category", "case": "first-letter"}
            },
            "namespacealiases": [{"id": 6, "alias": "Bild"}, {"id": 2, "alias": "Benutzerin"}]
        }})
    }

    #[test]
    fn parses_siteinfo() {
        let site = Site::from_siteinfo("de.wikipedia.org", &siteinfo()).unwrap();
        assert_eq!(site.dbname, "dewiki");
        assert_eq!(site.lang, "de");
        assert_eq!(site.file_prefixes, ["Datei", "File", "Bild"]);
        assert_eq!(site.template_prefixes, ["Vorlage", "Template"]);
        assert!(!site.template_case_sensitive);
        assert_eq!(site.full_title(14, "Deutsche_Person"), "Kategorie:Deutsche Person");
        assert_eq!(site.full_title(0, "Berlin"), "Berlin");
    }

    #[test]
    fn db_keys() {
        let site = Site::from_siteinfo("de.wikipedia.org", &siteinfo()).unwrap();
        assert_eq!(site.db_key(NS_CATEGORY, "Kategorie:deutsche  Person"), "Deutsche_Person");
        assert_eq!(site.db_key(NS_CATEGORY, "Category:A b"), "A_b");
        assert_eq!(site.db_key(NS_TEMPLATE, "Vorlage:Normdaten"), "Normdaten");
        // A category whose name merely looks prefixed (talk page example) is kept whole.
        assert_eq!(
            site.db_key(NS_CATEGORY, "Wikipedia:GND in Wikipedia vorhanden"),
            "Wikipedia:GND_in_Wikipedia_vorhanden"
        );
    }

    #[test]
    fn rejects_odd_dbnames() {
        let mut json = siteinfo();
        json["query"]["general"]["wikiid"] = json!("x; DROP TABLE");
        assert!(Site::from_siteinfo("x", &json).is_err());
    }
}
