//! One-off import of the old tool's shared queries, keeping their ids so
//! `?htid=…` links keep working. The old tool publishes them over HTTP.

use crate::harvest::JobSpec;
use crate::storage::{LegacyShare, Store};
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;
use std::time::Duration;

const LIST_URL: &str = "https://pltools.toolforge.org/harvesttemplates/share.php";
const SPEC_URL: &str = "https://pltools.toolforge.org/harvesttemplates/gethtshare.php";
/// Be gentle with the old tool.
const PAUSE: Duration = Duration::from_millis(250);

static ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<tr data-id="(\d+)">.*?/wiki/User:[^"]*">([^<]*)</a></td><td>([^<]*)</td>"#).unwrap()
});

/// An entry of the old list: id, creator, last complete run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: u64,
    pub user_name: String,
    pub last_run: Option<i64>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub listed: usize,
    pub imported: usize,
    pub already_there: usize,
    /// Id and reason, for shares that cannot work (no property or template).
    pub unusable: Vec<(u64, String)>,
}

/// Fetch and convert every old share; store them unless `store` is `None` (dry run).
pub async fn import(http: &reqwest::Client, store: Option<&Store>) -> Result<Report> {
    let html = http.get(LIST_URL).send().await?.error_for_status()?.text().await?;
    let listed = parse_list(&html);
    let mut report = Report { listed: listed.len(), ..Default::default() };
    for entry in listed {
        tokio::time::sleep(PAUSE).await;
        let json: Value = http
            .get(SPEC_URL)
            .query(&[("htid", entry.id)])
            .send()
            .await?
            .json()
            .await
            .with_context(|| format!("htid {}", entry.id))?;
        let spec = JobSpec::from_legacy_query(&pairs(&json));
        if spec.property.is_none() || spec.template.trim().is_empty() {
            report.unusable.push((entry.id, "no property or template".into()));
            continue;
        }
        let share = LegacyShare {
            legacy_id: entry.id,
            title: title(&spec),
            user_name: entry.user_name,
            spec,
            last_completed: entry.last_run,
        };
        match store {
            Some(store) if store.import_legacy_share(&share).await? => report.imported += 1,
            Some(_) => report.already_there += 1,
            None => report.imported += 1,
        }
    }
    Ok(report)
}

pub fn parse_list(html: &str) -> Vec<Listed> {
    ROW.captures_iter(html)
        .filter_map(|c| {
            let last_run = chrono::NaiveDateTime::parse_from_str(c[3].trim(), "%Y-%m-%d %H:%M:%S").ok();
            Some(Listed {
                id: c[1].parse().ok()?,
                user_name: c[2].replace("&amp;", "&").trim().to_string(),
                last_run: last_run.map(|t| t.and_utc().timestamp()),
            })
        })
        .collect()
}

/// The old tool stored the permalink query; PHP turned `a[]=1&a[]=2` into arrays.
fn pairs(json: &Value) -> Vec<(String, String)> {
    let fields = json.as_object().into_iter().flatten();
    fields
        .flat_map(|(key, value)| {
            let values: Vec<String> = match value {
                Value::Array(items) => items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Value::String(s) => vec![s.clone()],
                other => vec![other.to_string()],
            };
            values.into_iter().map(move |v| (key.clone(), v))
        })
        .collect()
}

/// The old shares had no titles: `Infobox film → P57 (en.wikipedia)`.
fn title(spec: &JobSpec) -> String {
    let property = spec.property.map(|p| p.to_string()).unwrap_or_default();
    format!("{} → {property} ({}.{})", spec.template, spec.siteid, spec.project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::PropertyId;
    use serde_json::json;

    #[test]
    fn list_rows() {
        let html = r#"<tr data-id="3"><td>en.wikipedia<br /><small>NS: 0</small><td><a href="//www.wikidata.org/wiki/Property:P57">P57</a></td><td>director</td><td><i>depth</i>: 0<br /><td><a href="//www.wikidata.org/wiki/User:Pasleim">Pasleim</a></td><td>2022-03-25 16:38:14</td><td><a href="index.html?htid=3">load</a></td></tr>
            <tr data-id="4"><td>x</td><td><a href="//www.wikidata.org/wiki/User:A &amp; B">A &amp; B</a></td><td></td><td></td></tr>"#;
        let listed = parse_list(html);
        assert_eq!(listed[0], Listed { id: 3, user_name: "Pasleim".into(), last_run: Some(1_648_226_294) });
        assert_eq!(listed[1], Listed { id: 4, user_name: "A & B".into(), last_run: None });
    }

    #[test]
    fn stored_queries() {
        let json = json!({"action": "savenew", "siteid": "en", "project": "wikipedia", "p": "P989",
            "template": "Spoken Wikipedia", "parameters": "1|2|3|", "depth": "3"});
        let spec = JobSpec::from_legacy_query(&pairs(&json));
        assert_eq!(spec.property, Some(PropertyId(989)));
        assert_eq!(spec.parameters, ["1", "2", "3"]);
        assert_eq!(title(&spec), "Spoken Wikipedia → P989 (en.wikipedia)");
        let php_array = json!({"property": "57", "template": "X", "parameter": ["director", "1"]});
        assert_eq!(JobSpec::from_legacy_query(&pairs(&php_array)).parameters, ["director", "1"]);
    }
}
