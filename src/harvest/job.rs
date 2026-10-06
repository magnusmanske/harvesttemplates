//! A validated spec, with everything looked up once per run.

use super::spec::JobSpec;
use super::spec::QualifierSource;
use crate::app_state::Clients;
use crate::constraints;
use crate::ids::{ItemId, PropertyId};
use crate::value::{self, Datatype, DecimalMark, Transform, Value, ValueError};
use crate::wiki::Site;
use crate::wiki::site::{NS_TEMPLATE, host_for};
use crate::wikidata::{ConstraintDef, ConstraintStatus, PropertyInfo};
use crate::wikitext::TemplateMatcher;
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// The spec cannot work; the message tells the user why.
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

fn invalid(msg: impl Into<String>) -> JobError {
    JobError::Invalid(msg.into())
}

#[derive(Debug)]
pub struct Job {
    pub spec: JobSpec,
    pub site: Site,
    pub property: PropertyInfo,
    pub datatype: Datatype,
    /// Database key of the template, e.g. `Infobox_person`.
    pub template_key: String,
    pub matcher: TemplateMatcher,
    pub transform: Transform,
    /// The constraints to check: those selected, plus all mandatory ones.
    pub constraints: Vec<ConstraintDef>,
    pub qualifiers: Vec<PreparedQualifier>,
    /// Quantities: unit names and symbols (lower case) → unit, for values like `82 g`.
    pub units: HashMap<String, ItemId>,
    /// More properties from the same template (#111), each prepared like a job of its own.
    pub extras: Vec<Job>,
}

#[derive(Debug, Clone)]
pub struct PreparedQualifier {
    pub property: PropertyId,
    pub datatype: Datatype,
    pub source: PreparedSource,
}

#[derive(Debug, Clone)]
pub enum PreparedSource {
    /// Parsed once, when the run is created.
    Fixed(Value),
    Parameter(Vec<String>),
}

impl Job {
    pub async fn prepare(clients: &Clients, spec: JobSpec) -> Result<Self, JobError> {
        let mut job = Self::prepare_one(clients, spec.clone(), true).await?;
        for extra in &spec.extra_properties {
            let extra_job = Self::prepare_one(clients, spec.for_extra(extra), false).await;
            job.extras.push(extra_job.map_err(|e| match e {
                JobError::Invalid(m) => invalid(format!("{}: {m}", extra.property)),
                other => other,
            })?);
        }
        Ok(job)
    }

    /// Field 0 is the main property, then the extras.
    pub fn field(&self, index: u8) -> Option<&Self> {
        match index {
            0 => Some(self),
            i => self.extras.get(usize::from(i) - 1),
        }
    }

    pub fn field_count(&self) -> u8 {
        u8::try_from(1 + self.extras.len()).unwrap_or(u8::MAX)
    }

    /// `main`: the unit must be allowed by the property. Extras have no unit of
    /// their own, so their values must name one (`82 g`).
    async fn prepare_one(clients: &Clients, spec: JobSpec, main: bool) -> Result<Self, JobError> {
        check_value_source(&spec)?;
        let transform = Transform::new(spec.transform.clone()).map_err(|e| invalid(format!("invalid regex: {e}")))?;
        let host = host_for(&spec.siteid, &spec.project).map_err(|e| invalid(e.to_string()))?;
        let mut site = Site::load(&clients.mw, &host).await.map_err(|_| invalid(format!("cannot reach {host}")))?;
        site.edition = clients.wdqs.edition_for(&site.dbname).await.unwrap_or_else(|e| {
            tracing::warn!("no edition item for {}: {e:#}", site.dbname);
            None
        });
        let property_id = spec.property.ok_or_else(|| invalid("choose a property"))?;
        let property = clients.wikidata.property(property_id).await?;
        let property = property.ok_or_else(|| invalid(format!("{property_id} does not exist")))?;
        let datatype = check_property(&property, &spec, main)?;
        let template_key = site.db_key(NS_TEMPLATE, &spec.template);
        let redirects = site.template_redirects(&clients.mw, &template_key).await?;
        let redirects = redirects.ok_or_else(|| invalid(format!("Template:{} does not exist", spec.template)))?;
        let matcher =
            site.template_matcher(template_names(&template_key, redirects, spec.template_redirects.as_deref()));
        let constraints = selected_constraints(&property, spec.constraints.as_deref());
        let qualifiers = prepare_qualifiers(clients, &spec).await?;
        let units = if datatype == Datatype::Quantity {
            unit_names(clients, &property, &spec, &site).await?
        } else {
            HashMap::new()
        };
        let extras = vec![];
        Ok(Self {
            spec,
            site,
            property,
            datatype,
            template_key,
            matcher,
            transform,
            constraints,
            qualifiers,
            units,
            extras,
        })
    }
}

/// Names of the units a value may carry: the allowed ones, or the chosen one.
/// Names shared by two units are dropped rather than guessed.
async fn unit_names(
    clients: &Clients,
    property: &PropertyInfo,
    spec: &JobSpec,
    site: &Site,
) -> Result<HashMap<String, ItemId>, JobError> {
    let candidates: Vec<ItemId> = match property.allowed_units() {
        Some(allowed) => allowed.into_iter().flatten().collect(),
        None => spec.unit.into_iter().collect(),
    };
    let names = clients.wikidata.names(&candidates, &format!("en|{}", site.lang)).await?;
    let mut map: HashMap<String, Option<ItemId>> = HashMap::new();
    for (unit, names) in names {
        for name in names {
            let entry = map.entry(value::unit_key(&name)).or_insert(Some(unit));
            if *entry != Some(unit) {
                *entry = None;
            }
        }
    }
    Ok(map.into_iter().filter_map(|(name, unit)| Some((name, unit?))).collect())
}

async fn prepare_qualifiers(clients: &Clients, spec: &JobSpec) -> Result<Vec<PreparedQualifier>, JobError> {
    let mut prepared = Vec::with_capacity(spec.qualifiers.len());
    for q in &spec.qualifiers {
        let info = clients.wikidata.property(q.property).await?;
        let info = info.ok_or_else(|| invalid(format!("qualifier {} does not exist", q.property)))?;
        let datatype = info
            .datatype
            .filter(|_| !info.deprecated)
            .ok_or_else(|| invalid(format!("qualifier {} ({}) is not supported", q.property, info.datatype_name)))?;
        let source = match &q.source {
            QualifierSource::Fixed { value } => PreparedSource::Fixed(
                fixed_value(datatype, value.trim(), spec)
                    .map_err(|e| invalid(format!("qualifier {}: '{value}': {e}", q.property)))?,
            ),
            QualifierSource::Parameter { names } if names.is_empty() => {
                return Err(invalid(format!("qualifier {}: choose a parameter", q.property)));
            }
            QualifierSource::Parameter { names } => PreparedSource::Parameter(names.clone()),
        };
        prepared.push(PreparedQualifier { property: q.property, datatype, source });
    }
    Ok(prepared)
}

/// A typed-in qualifier value. Items are ids; monolingual text may end in `@language`.
fn fixed_value(datatype: Datatype, text: &str, spec: &JobSpec) -> Result<Value, String> {
    let err = |e: ValueError| e.to_string();
    Ok(match datatype {
        Datatype::Item => Value::Item(text.parse()?),
        Datatype::Time => {
            Value::Time { date: value::parse_date(text, "en", spec.calendar).map_err(err)?, calendar: spec.calendar }
        }
        Datatype::Quantity => {
            Value::Quantity { amount: value::parse_amount(text, DecimalMark::Point).map_err(err)?, unit: None }
        }
        Datatype::Url => Value::String(value::url(text).map_err(err)?),
        Datatype::GlobeCoordinate => Value::Coordinate(value::parse_coordinate(text, &[]).map_err(err)?),
        Datatype::Monolingual => {
            let (text, language) = text.rsplit_once('@').ok_or("write it as text@language")?;
            check_language(language).map_err(|e| e.to_string())?;
            Value::Monolingual { text: text.to_string(), language: language.to_string() }
        }
        Datatype::String | Datatype::ExternalId | Datatype::CommonsMedia if !text.is_empty() => {
            Value::String(text.to_string())
        }
        _ => return Err("empty value".into()),
    })
}

fn check_value_source(spec: &JobSpec) -> Result<(), JobError> {
    if spec.template.trim().is_empty() {
        return Err(invalid("choose a template"));
    }
    if !spec.value_pattern.is_empty() && !(spec.value_pattern.contains('{') && spec.value_pattern.contains('}')) {
        return Err(invalid("the value pattern needs at least one {parameter}"));
    }
    if !spec.sparql.trim().is_empty() && !spec.sparql.contains("?item") {
        return Err(invalid("the SPARQL query must select ?item"));
    }
    let has_source = spec.use_page_title
        || !spec.value_pattern.is_empty()
        || spec.date_parameters.is_some()
        || spec.coordinate_parameters.is_some()
        || spec.parameters.iter().any(|p| !p.trim().is_empty());
    if !has_source {
        return Err(invalid("choose a template parameter"));
    }
    Ok(())
}

fn check_property(property: &PropertyInfo, spec: &JobSpec, check_units: bool) -> Result<Datatype, JobError> {
    if property.deprecated {
        return Err(invalid(format!("{} is deprecated", property.id)));
    }
    let datatype =
        property.datatype.ok_or_else(|| invalid(format!("datatype {} is not supported", property.datatype_name)))?;
    match datatype {
        Datatype::Quantity if check_units => check_unit(property, spec)?,
        Datatype::Monolingual if !spec.use_page_title || !spec.language.is_empty() => check_language(&spec.language)?,
        Datatype::Monolingual => return Err(invalid("choose a language code")),
        _ => {}
    }
    if spec.date_parameters.is_some() && datatype != Datatype::Time {
        return Err(invalid("year/month/day parameters only work for dates"));
    }
    if spec.coordinate_parameters.is_some() && datatype != Datatype::GlobeCoordinate {
        return Err(invalid("latitude/longitude parameters only work for coordinates"));
    }
    Ok(datatype)
}

/// Units must be allowed by the property's "allowed units" constraint, if it has one (#156, #178).
fn check_unit(property: &PropertyInfo, spec: &JobSpec) -> Result<(), JobError> {
    let Some(allowed) = property.allowed_units() else { return Ok(()) };
    if allowed.contains(&spec.unit) {
        return Ok(());
    }
    let unit = spec.unit.map_or_else(|| "no unit".to_string(), |u| u.to_string());
    Err(invalid(format!("{unit} is not an allowed unit for {}", property.id)))
}

fn check_language(code: &str) -> Result<(), JobError> {
    let valid = !code.is_empty()
        && code.len() <= 20
        && code.starts_with(|c: char| c.is_ascii_lowercase())
        && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid { Ok(()) } else { Err(invalid("choose a valid language code")) }
}

/// The template itself plus the accepted redirects (#159).
fn template_names(key: &str, redirects: Vec<String>, selected: Option<&[String]>) -> Vec<String> {
    let mut names = vec![key.replace('_', " ")];
    names.extend(redirects.into_iter().filter(|r| selected.is_none_or(|s| s.contains(r))));
    names
}

fn selected_constraints(property: &PropertyInfo, selected: Option<&[ItemId]>) -> Vec<ConstraintDef> {
    property
        .constraints
        .iter()
        .filter(|c| constraints::find(c.kind).is_some())
        .filter(|c| match c.status {
            ConstraintStatus::Mandatory => true,
            ConstraintStatus::Suggestion => false,
            ConstraintStatus::Normal => selected.is_none_or(|s| s.contains(&c.kind)),
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_names_respect_selection() {
        let redirects = || vec!["Death date".to_string(), "Event date and age".to_string()];
        assert_eq!(template_names("Death_date_and_age", redirects(), None).len(), 3);
        let only = ["Death date".to_string()];
        assert_eq!(
            template_names("Death_date_and_age", redirects(), Some(&only)),
            ["Death date and age", "Death date"]
        );
    }

    #[test]
    fn language_codes() {
        assert!(check_language("en").is_ok());
        assert!(check_language("zh-hant").is_ok());
        assert!(check_language("").is_err());
        assert!(check_language("EN").is_err());
        assert!(check_language("en<script>").is_err());
    }

    #[test]
    fn value_source_required() {
        let spec = JobSpec { template: "X".into(), ..Default::default() };
        assert!(check_value_source(&spec).is_err());
        assert!(check_value_source(&JobSpec { parameters: vec!["1".into()], ..spec.clone() }).is_ok());
        assert!(check_value_source(&JobSpec { use_page_title: true, ..spec }).is_ok());
    }
}
