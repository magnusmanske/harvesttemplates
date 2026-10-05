//! A validated spec, with everything looked up once per run.

use super::spec::JobSpec;
use crate::app_state::Clients;
use crate::wiki::Site;
use crate::wiki::site::{NS_TEMPLATE, host_for};
use crate::wikidata::{ConstraintDef, ConstraintStatus, PropertyInfo};
use crate::wikitext::TemplateMatcher;
use crate::{constraints, ids::ItemId, value::Datatype, value::Transform};

const ALLOWED_UNITS: ItemId = ItemId(21_514_353);

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
}

impl Job {
    pub async fn prepare(clients: &Clients, spec: JobSpec) -> Result<Self, JobError> {
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
        let datatype = check_property(&property, &spec)?;
        let template_key = site.db_key(NS_TEMPLATE, &spec.template);
        let redirects = site.template_redirects(&clients.mw, &template_key).await?;
        let redirects = redirects.ok_or_else(|| invalid(format!("Template:{} does not exist", spec.template)))?;
        let matcher =
            site.template_matcher(template_names(&template_key, redirects, spec.template_redirects.as_deref()));
        let constraints = selected_constraints(&property, spec.constraints.as_deref());
        Ok(Self { spec, site, property, datatype, template_key, matcher, transform, constraints })
    }
}

fn check_value_source(spec: &JobSpec) -> Result<(), JobError> {
    if spec.template.trim().is_empty() {
        return Err(invalid("choose a template"));
    }
    let has_source = spec.use_page_title || spec.date_parameters.is_some() || !spec.parameters.is_empty();
    if !has_source {
        return Err(invalid("choose a template parameter"));
    }
    Ok(())
}

fn check_property(property: &PropertyInfo, spec: &JobSpec) -> Result<Datatype, JobError> {
    if property.deprecated {
        return Err(invalid(format!("{} is deprecated", property.id)));
    }
    let datatype =
        property.datatype.ok_or_else(|| invalid(format!("datatype {} is not supported", property.datatype_name)))?;
    match datatype {
        Datatype::Quantity => check_unit(property, spec)?,
        Datatype::Monolingual if !spec.use_page_title || !spec.language.is_empty() => check_language(&spec.language)?,
        Datatype::Monolingual => return Err(invalid("choose a language code")),
        _ => {}
    }
    if spec.date_parameters.is_some() && datatype != Datatype::Time {
        return Err(invalid("year/month/day parameters only work for dates"));
    }
    Ok(datatype)
}

/// Units must be allowed by the property's "allowed units" constraint, if it has one (#156, #178).
fn check_unit(property: &PropertyInfo, spec: &JobSpec) -> Result<(), JobError> {
    let Some(constraint) = property.constraint(ALLOWED_UNITS) else {
        return Ok(());
    };
    let allowed: Vec<Option<ItemId>> = constraint
        .snaks(crate::ids::PropertyId(2305))
        .iter()
        .map(|s| s["datavalue"]["value"]["id"].as_str().and_then(|id| id.parse().ok()))
        .collect();
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
