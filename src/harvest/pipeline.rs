//! One page in, one planned edit (or a reason why not) out.
//! Every rejection message is shown to the user; see `docs/HARVEST_PIPELINE.md`.

use super::job::Job;
use super::spec::SkipIf;
use crate::app_state::Clients;
use crate::constraints::{Candidate, first_violation};
use crate::ids::ItemId;
use crate::value::{self, Datatype, Date, Value, ValueError};
use crate::wiki::Page;
use crate::wiki::content::{self, FileLocation, LinkTarget, Revision};
use crate::wikidata::{Entity, Source, statement};
use crate::wikitext::clean_value;
use serde_json::Value as Json;

/// Nothing to do (`Skip`), or something is wrong with the value (`Error`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    Skip(String),
    Error(String),
}

fn skip(msg: impl Into<String>) -> Rejection {
    Rejection::Skip(msg.into())
}

fn error(msg: impl Into<String>) -> Rejection {
    Rejection::Error(msg.into())
}

fn bad_value(e: ValueError) -> Rejection {
    error(e.to_string())
}

fn failed(e: anyhow::Error) -> Rejection {
    error(format!("lookup failed: {e:#}"))
}

#[derive(Debug, Clone)]
pub struct PlannedEdit {
    /// The item to edit; differs from the page's item if that was merged.
    pub item: ItemId,
    pub value: Value,
    pub statement: Json,
}

#[derive(Debug)]
pub struct Outcome {
    /// The template value as found, for the results table.
    pub raw: Option<String>,
    /// The parsed value, for the results table.
    pub value: Option<String>,
    pub result: Result<PlannedEdit, Rejection>,
}

pub async fn evaluate(job: &Job, clients: &Clients, page: &Page, revision: &Revision) -> Outcome {
    let mut outcome = Outcome {
        raw: None,
        value: None,
        result: Err(skip("")),
    };
    outcome.result = steps(job, clients, page, revision, &mut outcome).await;
    outcome
}

async fn steps(
    job: &Job,
    clients: &Clients,
    page: &Page,
    revision: &Revision,
    out: &mut Outcome,
) -> Result<PlannedEdit, Rejection> {
    let item = page.item.ok_or_else(|| skip("the page has no Wikidata item"))?;
    let raw = extract(job, page, &revision.text)?;
    out.raw = Some(raw.to_string());
    let value = parse(job, clients, &raw, item).await?;
    out.value = Some(value.display());
    let entity = clients.wikidata.item(item).await.map_err(failed)?;
    let entity = entity.ok_or_else(|| error("the item does not exist"))?;
    check_existing(job, &entity, &value)?;
    let candidate = Candidate {
        item: &entity,
        property: job.property.id,
        datatype: job.datatype,
        value: &value,
        qualifiers: &[],
    };
    let defs: Vec<_> = job.constraints.iter().collect();
    if let Some(name) = first_violation(&defs, &candidate, clients.services())
        .await
        .map_err(failed)?
    {
        return Err(error(format!("constraint violation: {name}")));
    }
    let source = Source::new(job.site.edition, &job.site.host, &page.title, revision.id);
    let statement = statement(job.property.id, job.datatype, &value, &source);
    Ok(PlannedEdit {
        item: entity.id,
        value,
        statement,
    })
}

#[derive(Debug)]
enum RawValue {
    Text(String),
    DateParts {
        year: String,
        month: Option<String>,
        day: Option<String>,
    },
}

impl std::fmt::Display for RawValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(t) => f.write_str(t),
            Self::DateParts { year, month, day } => {
                let parts = [Some(year), month.as_ref(), day.as_ref()];
                f.write_str(&parts.into_iter().flatten().cloned().collect::<Vec<_>>().join(" / "))
            }
        }
    }
}

fn extract(job: &Job, page: &Page, wikitext: &str) -> Result<RawValue, Rejection> {
    if job.spec.use_page_title {
        let prefix = job.site.namespaces.get(&job.spec.namespace).map(|ns| format!("{ns}:"));
        let title = prefix.and_then(|p| page.title.strip_prefix(&p)).unwrap_or(&page.title);
        return Ok(RawValue::Text(title.to_string()));
    }
    let params = job.matcher.find(wikitext).ok_or_else(|| skip("template not found"))?;
    if let Some(dp) = &job.spec.date_parameters {
        let get = |name: &Option<String>| name.as_deref().and_then(|n| params.get(n)).map(clean_value);
        let year = params.get(&dp.year).map(clean_value).ok_or_else(|| skip("no value"))?;
        return Ok(RawValue::DateParts {
            year,
            month: get(&dp.month),
            day: get(&dp.day),
        });
    }
    let raw = params
        .first_of(job.spec.parameters.iter().map(String::as_str))
        .ok_or_else(|| skip("no value"))?;
    Ok(RawValue::Text(clean_value(raw)))
}

async fn parse(job: &Job, clients: &Clients, raw: &RawValue, item: ItemId) -> Result<Value, Rejection> {
    let lang = &job.site.lang;
    let calendar = job.spec.calendar;
    let text = match raw {
        RawValue::DateParts { year, month, day } => {
            let date = value::parse_date_parts(year, month.as_deref(), day.as_deref(), lang, calendar);
            return time_value(job, date.map_err(bad_value)?);
        }
        RawValue::Text(text) => job.transform.apply(text).trim().to_string(),
    };
    if text.is_empty() {
        return Err(skip("no value"));
    }
    match job.datatype {
        Datatype::Item => resolve_item(job, clients, &text, item).await,
        Datatype::CommonsMedia => commons_file(job, clients, &text).await,
        Datatype::Url => value::url(&text).map(Value::String).map_err(bad_value),
        Datatype::String | Datatype::ExternalId => Ok(Value::String(text)),
        Datatype::Time => time_value(job, value::parse_date(&text, lang, calendar).map_err(bad_value)?),
        Datatype::Quantity => value::parse_amount(&text, job.spec.decimal_mark)
            .map(|amount| Value::Quantity {
                amount,
                unit: job.spec.unit,
            })
            .map_err(bad_value),
        Datatype::Monolingual => Ok(Value::Monolingual {
            text,
            language: job.spec.language.clone(),
        }),
    }
}

fn time_value(job: &Job, date: Date) -> Result<Value, Rejection> {
    if job.spec.date_limit.is_some_and(|limit| !limit.accepts(&date)) {
        return Err(bad_value(ValueError::OutsideDateLimit));
    }
    Ok(Value::Time {
        date,
        calendar: job.spec.calendar,
    })
}

async fn resolve_item(job: &Job, clients: &Clients, text: &str, item: ItemId) -> Result<Value, Rejection> {
    let title = value::link_target(text, job.spec.plain_links, job.spec.link_choice).map_err(bad_value)?;
    match content::link_target(&clients.mw, &job.site, &title)
        .await
        .map_err(failed)?
    {
        LinkTarget::Item(q) if q == item => Err(error("the link points to the page itself")),
        LinkTarget::Item(q) => Ok(Value::Item(q)),
        LinkTarget::NoItem => Err(error(format!("[[{title}]] has no Wikidata item"))),
        LinkTarget::Missing => Err(error(format!("[[{title}]] does not exist"))),
    }
}

async fn commons_file(job: &Job, clients: &Clients, text: &str) -> Result<Value, Rejection> {
    let name = value::file_name(text, &job.site.file_prefixes).map_err(bad_value)?;
    match content::file_location(&clients.mw, &job.site, &name)
        .await
        .map_err(failed)?
    {
        FileLocation::Commons => Ok(Value::String(name)),
        FileLocation::Local => Err(error(format!(
            "the file is only on {}, not on Commons",
            job.site.dbname
        ))),
        FileLocation::Missing => Err(error("the file does not exist")),
    }
}

/// Checked against the item as it is now, not a (lagging) query service.
fn check_existing(job: &Job, entity: &Entity, value: &Value) -> Result<(), Rejection> {
    let property = job.property.id;
    if entity.has_value(property, value) {
        return Err(skip("the item already has this value"));
    }
    if job.spec.skip_if == SkipIf::Property && entity.has_property(property) {
        return Err(skip("the item already has the property"));
    }
    Ok(())
}

/// Says what was added, and links the edit group so the whole run can be reviewed or undone (#175).
pub fn summary(job: &Job, value: &Value, editgroup: &str) -> String {
    let shown = match value {
        Value::Item(q) => format!("[[{q}]]"),
        other => other.display().chars().take(100).collect(),
    };
    format!(
        "Added [[Property:{}]]: {shown} from {} ([[:toollabs:editgroups/b/harvesttemplates/{editgroup}|details]])",
        job.property.id, job.site.dbname
    )
}

#[cfg(test)]
mod tests;
