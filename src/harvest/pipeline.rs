//! One page in, one planned edit (or a reason why not) out.
//! Every rejection message is shown to the user; see `docs/HARVEST_PIPELINE.md`.

use super::job::{Job, PreparedSource};
use super::spec::SkipIf;
use crate::app_state::Clients;
use crate::constraints::{Candidate, first_violation};
use crate::ids::ItemId;
use crate::value::{self, Datatype, Date, Value, ValueError};
use crate::wiki::Page;
use crate::wiki::content::{self, FileLocation, LinkTarget, Revision};
use crate::wikidata::{Entity, Qualifier, Source, statement};
use crate::wikitext::{TemplateParams, clean_value_with, lead_section};
use regex::Regex;
use serde_json::Value as Json;
use std::sync::LazyLock;

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
    let mut outcome = Outcome { raw: None, value: None, result: Err(skip("")) };
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
    let found = extract(job, page, &revision.text)?;
    out.raw = Some(found.raw.to_string());
    let value = parse(job, clients, &found.raw, item).await?;
    let qualifiers = qualifiers(job, clients, found.params.as_ref(), item).await?;
    out.value = Some(display(&value, &qualifiers));
    let entity = clients.wikidata.item(item).await.map_err(failed)?;
    let entity = entity.ok_or_else(|| error("the item does not exist"))?;
    check_existing(job, &entity, &value)?;
    let qualifier_ids: Vec<_> = qualifiers.iter().map(|q| q.property).collect();
    let candidate = Candidate {
        item: &entity,
        property: job.property.id,
        datatype: job.datatype,
        value: &value,
        qualifiers: &qualifier_ids,
    };
    let defs: Vec<_> = job.constraints.iter().collect();
    if let Some(name) = first_violation(&defs, &candidate, clients.services()).await.map_err(failed)? {
        return Err(error(format!("constraint violation: {name}")));
    }
    let source = Source::new(job.site.edition, &job.site.host, &page.title, revision.id);
    let statement = statement(job.property.id, job.datatype, &value, &qualifiers, &source);
    Ok(PlannedEdit { item: entity.id, value, statement })
}

#[derive(Debug)]
enum RawValue {
    Text(String),
    DateParts { year: String, month: Option<String>, day: Option<String> },
    CoordinateParts { latitude: String, longitude: String },
}

impl std::fmt::Display for RawValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(t) => f.write_str(t),
            Self::DateParts { year, month, day } => {
                let parts = [Some(year), month.as_ref(), day.as_ref()];
                f.write_str(&parts.into_iter().flatten().cloned().collect::<Vec<_>>().join(" / "))
            }
            Self::CoordinateParts { latitude, longitude } => write!(f, "{latitude} / {longitude}"),
        }
    }
}

/// The main raw value, and the template's parameters (qualifiers may need them).
struct Found {
    raw: RawValue,
    params: Option<TemplateParams>,
}

fn extract(job: &Job, page: &Page, wikitext: &str) -> Result<Found, Rejection> {
    let spec = &job.spec;
    let params = job.matcher.find(if spec.lead_only { lead_section(wikitext) } else { wikitext });
    let clean = |raw: &str| clean_value_with(raw, spec.unwrap_templates);
    if job.spec.use_page_title {
        let prefix = job.site.namespaces.get(&job.spec.namespace).map(|ns| format!("{ns}:"));
        let title = prefix.and_then(|p| page.title.strip_prefix(&p)).unwrap_or(&page.title);
        return Ok(Found { raw: RawValue::Text(title.to_string()), params });
    }
    let p = params.as_ref().ok_or_else(|| skip("template not found"))?;
    let raw = if let Some(dp) = &job.spec.date_parameters {
        let get = |name: &Option<String>| name.as_deref().and_then(|n| p.get(n)).map(clean);
        let year = p.get(&dp.year).map(clean).ok_or_else(|| skip("no value"))?;
        RawValue::DateParts { year, month: get(&dp.month), day: get(&dp.day) }
    } else if let Some(cp) = &job.spec.coordinate_parameters {
        let get = |name: &str| p.get(name).map(str::to_string).ok_or_else(|| skip("no value"));
        RawValue::CoordinateParts { latitude: get(&cp.latitude)?, longitude: get(&cp.longitude)? }
    } else if !spec.value_pattern.is_empty() {
        RawValue::Text(fill_pattern(&spec.value_pattern, p, &clean).ok_or_else(|| skip("no value"))?)
    } else if job.datatype == Datatype::GlobeCoordinate {
        RawValue::Text(coordinate_text(p, &job.spec.parameters).ok_or_else(|| skip("no value"))?)
    } else {
        let raw = p.first_of(job.spec.parameters.iter().map(String::as_str)).map(clean);
        // Before any transform: "add prefix" must not turn nothing into something.
        RawValue::Text(raw.filter(|r| !r.is_empty()).ok_or_else(|| skip("no value"))?)
    };
    Ok(Found { raw, params })
}

/// `{1}-{2}` with each placeholder replaced by that (cleaned) parameter; `None` if one is missing.
fn fill_pattern(pattern: &str, params: &TemplateParams, clean: &impl Fn(&str) -> String) -> Option<String> {
    let mut complete = true;
    let filled = PLACEHOLDER.replace_all(pattern, |caps: &regex::Captures| {
        let value = params.get(&caps[1]).map(clean).filter(|v| !v.is_empty());
        complete &= value.is_some();
        value.unwrap_or_default()
    });
    complete.then(|| filled.into_owned())
}

static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([^{}]+)\}").unwrap());

/// The raw (uncleaned) value, so a nested `{{coord}}` survives. An unnamed
/// parameter brings the following unnamed ones along: `{{coord|52|31|N|13|24|E}}`.
fn coordinate_text(params: &TemplateParams, names: &[String]) -> Option<String> {
    let name = names.iter().find(|n| params.get(n).is_some())?;
    let Ok(first) = name.trim().parse::<usize>() else { return params.get(name).map(str::to_string) };
    let run: Vec<&str> = (first..).map_while(|i| params.get(&i.to_string())).collect();
    Some(run.join(" "))
}

async fn parse(job: &Job, clients: &Clients, raw: &RawValue, item: ItemId) -> Result<Value, Rejection> {
    let text = match raw {
        RawValue::CoordinateParts { latitude, longitude } => {
            return value::parse_coordinate_parts(latitude, longitude).map(Value::Coordinate).map_err(bad_value);
        }
        RawValue::DateParts { year, month, day } => {
            let date =
                value::parse_date_parts(year, month.as_deref(), day.as_deref(), &job.site.lang, job.spec.calendar);
            return time_value(job, date.map_err(bad_value)?);
        }
        RawValue::Text(text) => job.transform.apply(text).trim().to_string(),
    };
    if text.is_empty() {
        return Err(skip("no value"));
    }
    parse_as(job, clients, job.datatype, job.spec.unit, text, item).await
}

/// Text to a value of `datatype`, using the spec's calendar, decimal mark and language.
async fn parse_as(
    job: &Job,
    clients: &Clients,
    datatype: Datatype,
    unit: Option<ItemId>,
    text: String,
    item: ItemId,
) -> Result<Value, Rejection> {
    let spec = &job.spec;
    match datatype {
        Datatype::Item => resolve_item(job, clients, &text, item).await,
        Datatype::CommonsMedia => commons_file(job, clients, &text).await,
        Datatype::Url => value::url(&text).map(Value::String).map_err(bad_value),
        Datatype::String | Datatype::ExternalId => Ok(Value::String(text)),
        Datatype::Time => time_value(job, value::parse_date(&text, &job.site.lang, spec.calendar).map_err(bad_value)?),
        Datatype::Quantity => {
            let (number, suffix) = value::split_unit(&text);
            let amount = value::parse_amount(&number, spec.decimal_mark).map_err(bad_value)?;
            let unit = match suffix {
                None => unit,
                Some(s) => {
                    Some(*job.units.get(&value::unit_key(&s)).ok_or_else(|| error(format!("unknown unit '{s}'")))?)
                }
            };
            Ok(Value::Quantity { amount, unit })
        }
        Datatype::Monolingual => Ok(Value::Monolingual { text, language: spec.language.clone() }),
        Datatype::GlobeCoordinate => {
            value::parse_coordinate(&text, &job.site.template_prefixes).map(Value::Coordinate).map_err(bad_value)
        }
    }
}

/// Qualifier values for this page. A missing parameter just leaves its qualifier out;
/// an unusable one rejects the row, so no half-right statement is written.
async fn qualifiers(
    job: &Job,
    clients: &Clients,
    params: Option<&TemplateParams>,
    item: ItemId,
) -> Result<Vec<Qualifier>, Rejection> {
    let mut out = Vec::with_capacity(job.qualifiers.len());
    for q in &job.qualifiers {
        let value = match &q.source {
            PreparedSource::Fixed(value) => value.clone(),
            PreparedSource::Parameter(names) => {
                let raw = params.and_then(|p| p.first_of(names.iter().map(String::as_str)));
                let raw = raw.map(|r| clean_value_with(r, job.spec.unwrap_templates));
                let Some(text) = raw.filter(|t| !t.is_empty()) else { continue };
                let parsed = parse_as(job, clients, q.datatype, None, text, item).await;
                parsed.map_err(|r| match r {
                    Rejection::Skip(m) | Rejection::Error(m) => error(format!("qualifier {}: {m}", q.property)),
                })?
            }
        };
        out.push(Qualifier { property: q.property, datatype: q.datatype, value });
    }
    Ok(out)
}

fn display(value: &Value, qualifiers: &[Qualifier]) -> String {
    let mut shown = value.display();
    for q in qualifiers {
        shown.push_str(&format!("; {}: {}", q.property, q.value.display()));
    }
    shown
}

fn time_value(job: &Job, date: Date) -> Result<Value, Rejection> {
    if job.spec.date_limit.is_some_and(|limit| !limit.accepts(&date)) {
        return Err(bad_value(ValueError::OutsideDateLimit));
    }
    Ok(Value::Time { date, calendar: job.spec.calendar })
}

async fn resolve_item(job: &Job, clients: &Clients, text: &str, item: ItemId) -> Result<Value, Rejection> {
    let title = value::link_target(text, job.spec.plain_links, job.spec.link_choice).map_err(bad_value)?;
    match content::link_target(&clients.mw, &job.site, &title).await.map_err(failed)? {
        LinkTarget::Item(q) if q == item => Err(error("the link points to the page itself")),
        LinkTarget::Item(q) => Ok(Value::Item(q)),
        LinkTarget::NoItem => Err(error(format!("[[{title}]] has no Wikidata item"))),
        LinkTarget::Missing => Err(error(format!("[[{title}]] does not exist"))),
    }
}

async fn commons_file(job: &Job, clients: &Clients, text: &str) -> Result<Value, Rejection> {
    let name = value::file_name(text, &job.site.file_prefixes).map_err(bad_value)?;
    match content::file_location(&clients.mw, &job.site, &name).await.map_err(failed)? {
        FileLocation::Commons => Ok(Value::String(name)),
        FileLocation::Local => Err(error(format!("the file is only on {}, not on Commons", job.site.dbname))),
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
