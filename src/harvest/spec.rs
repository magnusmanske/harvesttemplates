//! What to harvest. Serialised as JSON for the API and storage, and convertible
//! to and from the original tool's permalink query string.

use crate::ids::{ItemId, PropertyId};
use crate::value::{ArchiveUrls, Calendar, Case, Date, DateLimit, DecimalMark, LinkChoice, Relation, TransformSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkipIf {
    /// Skip items that have any value for the property ("do not load items with property set").
    #[default]
    Property,
    /// Skip only items that already have this exact value (#40).
    Value,
}

/// A qualifier added to every harvested statement (#210, #133).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualifierSpec {
    pub property: PropertyId,
    #[serde(flatten)]
    pub source: QualifierSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum QualifierSource {
    /// The same value on every statement, e.g. `Q1860`.
    Fixed { value: String },
    /// A parameter (or aliases) of the same template transclusion.
    Parameter { names: Vec<String> },
}

impl QualifierSpec {
    /// Permalink form: `P407|fixed|Q1860` or `P585|param|date,datum`.
    fn to_legacy(&self) -> String {
        match &self.source {
            QualifierSource::Fixed { value } => format!("{}|fixed|{value}", self.property),
            QualifierSource::Parameter { names } => format!("{}|param|{}", self.property, names.join(",")),
        }
    }

    fn from_legacy(s: &str) -> Option<Self> {
        let mut parts = s.splitn(3, '|');
        let property = parts.next()?.parse().ok()?;
        let source = match (parts.next()?, parts.next()?.trim()) {
            ("fixed", value) => QualifierSource::Fixed { value: value.to_string() },
            ("param", names) => QualifierSource::Parameter { names: split_commas(names) },
            _ => return None,
        };
        Some(Self { property, source })
    }
}

/// Separate template parameters for latitude and longitude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateParameters {
    pub latitude: String,
    pub longitude: String,
}

/// Separate template parameters for year, month and day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateParameters {
    pub year: String,
    pub month: Option<String>,
    pub day: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobSpec {
    pub siteid: String,
    pub project: String,
    pub namespace: i32,
    pub property: Option<PropertyId>,
    pub template: String,
    /// Redirect names to accept; `None` accepts all redirects to the template.
    pub template_redirects: Option<Vec<String>>,
    /// Parameter name and aliases; the first one with a value wins.
    pub parameters: Vec<String>,
    pub date_parameters: Option<DateParameters>,
    pub coordinate_parameters: Option<CoordinateParameters>,
    /// Use the page title instead of a parameter.
    pub use_page_title: bool,
    /// Combine parameters, e.g. `{1}-{2}` (#52). Replaces `parameters` when set.
    pub value_pattern: String,
    /// Keep the first unnamed parameter of nested templates: `{{URL|x}}` → `x` (#2).
    pub unwrap_templates: bool,
    /// Only look before the first section heading (#122).
    pub lead_only: bool,
    pub transform: TransformSpec,
    /// URLs: what to do with links to archived copies.
    pub archive_urls: ArchiveUrls,
    /// Items: accept a value without `[[link]]` syntax as a page title.
    pub plain_links: bool,
    pub link_choice: LinkChoice,
    pub calendar: Calendar,
    pub date_limit: Option<DateLimit>,
    /// Quantities: `None` means "no unit".
    pub unit: Option<ItemId>,
    pub decimal_mark: DecimalMark,
    /// Monolingual text: language code.
    pub language: String,
    pub category: String,
    pub depth: u32,
    /// Page titles and/or item ids to restrict the run to.
    pub manual_list: Vec<String>,
    pub skip_if: SkipIf,
    /// Constraints to check; `None` checks all. Mandatory ones are always checked.
    pub constraints: Option<Vec<ItemId>>,
    pub qualifiers: Vec<QualifierSpec>,
}

/// The original tool's default: only Gregorian-safe dates.
const DEFAULT_LIMIT: DateLimit = DateLimit { relation: Relation::AtLeast, date: Date { year: 1926, month: 0, day: 0 } };

impl Default for JobSpec {
    fn default() -> Self {
        Self {
            siteid: "en".into(),
            project: "wikipedia".into(),
            namespace: 0,
            property: None,
            template: String::new(),
            template_redirects: None,
            parameters: vec![],
            date_parameters: None,
            coordinate_parameters: None,
            use_page_title: false,
            value_pattern: String::new(),
            unwrap_templates: false,
            lead_only: false,
            transform: TransformSpec::default(),
            archive_urls: ArchiveUrls::Original,
            plain_links: true,
            link_choice: LinkChoice::First,
            calendar: Calendar::Gregorian,
            date_limit: Some(DEFAULT_LIMIT),
            unit: None,
            decimal_mark: DecimalMark::Point,
            language: String::new(),
            category: String::new(),
            depth: 0,
            manual_list: vec![],
            skip_if: SkipIf::Property,
            constraints: None,
            qualifiers: vec![],
        }
    }
}

impl JobSpec {
    /// Parse an old permalink, e.g. `?siteid=de&project=wikipedia&p=227&template=Normdaten&parameters=GND`.
    /// Unknown keys (`htid`, `run`, …) are ignored.
    pub fn from_legacy_query(pairs: &[(String, String)]) -> Self {
        let mut spec = Self::default();
        let mut legacy_limit = (None, None);
        for (key, value) in pairs {
            let v = value.trim();
            match key.as_str() {
                "siteid" => spec.siteid = v.to_lowercase(),
                "project" => spec.project = v.to_lowercase(),
                "namespace" => spec.namespace = v.parse().unwrap_or(0),
                "p" | "property" => spec.property = v.parse().ok(),
                "template" => spec.template = v.to_string(),
                "templateredirects" => spec.template_redirects = Some(split_pipes(v)),
                "parameters" => spec.parameters.extend(split_pipes(v)),
                "parameter" if !v.is_empty() => spec.parameters.push(v.to_string()),
                "aparameter1" if !v.is_empty() => date_parameters(&mut spec).year = v.to_string(),
                "aparameter2" if !v.is_empty() => {
                    date_parameters(&mut spec).month = Some(v.to_string());
                }
                "aparameter3" if !v.is_empty() => {
                    date_parameters(&mut spec).day = Some(v.to_string());
                }
                "latparam" if !v.is_empty() => coordinate_parameters(&mut spec).latitude = v.to_string(),
                "lonparam" if !v.is_empty() => coordinate_parameters(&mut spec).longitude = v.to_string(),
                "pattern" => spec.value_pattern = value.clone(),
                "archive" => {
                    spec.archive_urls = match v {
                        "skip" => ArchiveUrls::Skip,
                        "keep" => ArchiveUrls::Keep,
                        _ => ArchiveUrls::Original,
                    }
                }
                "unwrap" => spec.unwrap_templates = v == "1",
                "lead" => spec.lead_only = v == "1",
                "case" => {
                    spec.transform.case = match v {
                        "lower" => Case::Lower,
                        "upper" => Case::Upper,
                        _ => Case::Unchanged,
                    }
                }
                "pagetitle" => spec.use_page_title = v == "1",
                "addprefix" | "prefix" => spec.transform.add_prefix = value.clone(),
                "addsuffix" => spec.transform.add_suffix = value.clone(),
                "removeprefix" => spec.transform.remove_prefix = value.clone(),
                "removesuffix" => spec.transform.remove_suffix = value.clone(),
                "searchvalue" => spec.transform.search = value.clone(),
                "replacevalue" => spec.transform.replace = value.clone(),
                "wikisyntax" => spec.plain_links = v == "1",
                "link" => spec.link_choice = if v == "last" { LinkChoice::Last } else { LinkChoice::First },
                "calendar" if v == Calendar::Julian.item().to_string() => {
                    spec.calendar = Calendar::Julian;
                }
                "limityear" => legacy_limit.0 = v.parse().ok(),
                "rel" => {
                    legacy_limit.1 = Some(if v == "l" { Relation::Before } else { Relation::AtLeast });
                }
                "unit" => spec.unit = v.parse().ok().filter(|_| v != "1"),
                "decimalmark" if v == "," => spec.decimal_mark = DecimalMark::Comma,
                "monolanguage" => spec.language = v.to_string(),
                "category" => spec.category = v.to_string(),
                "depth" => spec.depth = v.parse().unwrap_or(0),
                "manuallist" => {
                    spec.manual_list = v.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
                }
                "alreadyset" | "set" if v == "0" => spec.skip_if = SkipIf::Value,
                "skipif" if v == "value" => spec.skip_if = SkipIf::Value,
                "constraints" => {
                    spec.constraints = Some(split_pipes(v).iter().filter_map(|c| c.parse().ok()).collect());
                }
                "qualifier" => spec.qualifiers.extend(QualifierSpec::from_legacy(v)),
                _ => {}
            }
        }
        if let (Some(year), relation) = legacy_limit {
            let date = Date { year, month: 0, day: 0 };
            spec.date_limit = Some(DateLimit { relation: relation.unwrap_or(Relation::AtLeast), date });
        }
        spec.parameters.retain(|p| !p.trim().is_empty());
        spec
    }

    /// A permalink query string in the original tool's format (plus our extensions).
    pub fn to_legacy_query(&self) -> String {
        let d = Self::default();
        let t = &self.transform;
        let mut q: Vec<(&str, String)> = vec![
            ("siteid", self.siteid.clone()),
            ("project", self.project.clone()),
            ("namespace", self.namespace.to_string()),
        ];
        q.extend(self.property.map(|p| ("p", p.to_string())));
        q.push(("template", self.template.clone()));
        q.extend(self.template_redirects.as_ref().map(|r| ("templateredirects", r.join("|"))));
        q.push(("parameters", self.parameters.join("|")));
        if let Some(dp) = &self.date_parameters {
            q.push(("aparameter1", dp.year.clone()));
            q.extend(dp.month.clone().map(|m| ("aparameter2", m)));
            q.extend(dp.day.clone().map(|day| ("aparameter3", day)));
        }
        if let Some(cp) = &self.coordinate_parameters {
            q.push(("latparam", cp.latitude.clone()));
            q.push(("lonparam", cp.longitude.clone()));
        }

        for (key, value) in [
            ("addprefix", &t.add_prefix),
            ("addsuffix", &t.add_suffix),
            ("removeprefix", &t.remove_prefix),
            ("removesuffix", &t.remove_suffix),
            ("searchvalue", &t.search),
            ("replacevalue", &t.replace),
            ("monolanguage", &self.language),
            ("category", &self.category),
        ] {
            if !value.is_empty() {
                q.push((key, value.clone()));
            }
        }
        if self.depth != d.depth {
            q.push(("depth", self.depth.to_string()));
        }
        if !self.manual_list.is_empty() {
            q.push(("manuallist", self.manual_list.join("\n")));
        }
        q.push(("alreadyset", bit(self.skip_if == SkipIf::Property)));
        q.push(("wikisyntax", bit(self.plain_links)));
        if self.use_page_title {
            q.push(("pagetitle", "1".into()));
        }
        if !self.value_pattern.is_empty() {
            q.push(("pattern", self.value_pattern.clone()));
        }
        match self.archive_urls {
            ArchiveUrls::Original => {}
            ArchiveUrls::Skip => q.push(("archive", "skip".into())),
            ArchiveUrls::Keep => q.push(("archive", "keep".into())),
        }
        for (key, on) in [("unwrap", self.unwrap_templates), ("lead", self.lead_only)] {
            if on {
                q.push((key, "1".into()));
            }
        }
        match t.case {
            Case::Unchanged => {}
            Case::Lower => q.push(("case", "lower".into())),
            Case::Upper => q.push(("case", "upper".into())),
        }
        if self.link_choice == LinkChoice::Last {
            q.push(("link", "last".into()));
        }
        if self.calendar == Calendar::Julian {
            q.push(("calendar", Calendar::Julian.item().to_string()));
        }
        if let Some(limit) = self.date_limit.filter(|l| *l != DEFAULT_LIMIT) {
            q.push(("limityear", limit.date.year.to_string()));
            q.push(("rel", if limit.relation == Relation::Before { "l" } else { "geq" }.into()));
        }
        q.extend(self.unit.map(|u| ("unit", u.to_string())));
        if self.decimal_mark == DecimalMark::Comma {
            q.push(("decimalmark", ",".into()));
        }
        if let Some(c) = &self.constraints {
            q.push(("constraints", c.iter().map(ItemId::to_string).collect::<Vec<_>>().join("|")));
        }
        q.extend(self.qualifiers.iter().map(|qs| ("qualifier", qs.to_legacy())));
        let pairs: Vec<String> = q.into_iter().map(|(k, v)| format!("{k}={}", urlencoding::encode(&v))).collect();
        pairs.join("&")
    }
}

fn date_parameters(spec: &mut JobSpec) -> &mut DateParameters {
    spec.date_parameters.get_or_insert_with(|| DateParameters { year: String::new(), month: None, day: None })
}

fn split_pipes(s: &str) -> Vec<String> {
    s.split('|').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect()
}

fn coordinate_parameters(spec: &mut JobSpec) -> &mut CoordinateParameters {
    spec.coordinate_parameters
        .get_or_insert_with(|| CoordinateParameters { latitude: String::new(), longitude: String::new() })
}

fn split_commas(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect()
}

fn bit(b: bool) -> String {
    if b { "1" } else { "0" }.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(query: &str) -> JobSpec {
        let pairs: Vec<(String, String)> = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), urlencoding::decode(&v.replace('+', " ")).unwrap().into_owned()))
            .collect();
        JobSpec::from_legacy_query(&pairs)
    }

    /// Example 2 from the Wikidata talk page (GND from de-WP).
    #[test]
    fn talk_page_permalink() {
        let spec = parse(
            "siteid=de&project=wikipedia&namespace=0&p=P227&template=Normdaten&templateredirects=Authority%20control&parameters=GND&category=Wikipedia%3AGND%20in%20Wikipedia%20vorhanden%2C%20fehlt%20jedoch%20in%20Wikidata&depth=1&constraints=Q21502838%7CQ21502410%7CQ21510851%7CQ53869507%7CQ21502404&alreadyset=1&wikisyntax=1",
        );
        assert_eq!(spec.siteid, "de");
        assert_eq!(spec.property, Some(PropertyId(227)));
        assert_eq!(spec.template_redirects, Some(vec!["Authority control".to_string()]));
        assert_eq!(spec.parameters, ["GND"]);
        assert_eq!(spec.category, "Wikipedia:GND in Wikipedia vorhanden, fehlt jedoch in Wikidata");
        assert_eq!(spec.depth, 1);
        assert_eq!(spec.constraints.as_ref().map(Vec::len), Some(5));
        assert_eq!(spec.skip_if, SkipIf::Property);
    }

    #[test]
    fn oldest_permalink_format() {
        let spec = parse(
            "siteid=en&project=wikipedia&namespace=0&property=345&template=IMDb%20title&parameter=id&parameter=1&prefix=tt&category=2016%20films&depth=0&set=0&offset=0&limit=10000&wikisyntax=0&",
        );
        assert_eq!(spec.property, Some(PropertyId(345)));
        assert_eq!(spec.parameters, ["id", "1"]);
        assert_eq!(spec.transform.add_prefix, "tt");
        assert_eq!(spec.skip_if, SkipIf::Value);
        assert!(!spec.plain_links);
    }

    #[test]
    fn dates_and_quantities() {
        let spec = parse("p=569&template=Persondata&parameters=birth&calendar=Q1985786&limityear=1582&rel=l");
        assert_eq!(spec.calendar, Calendar::Julian);
        let limit = spec.date_limit.unwrap();
        assert_eq!((limit.relation, limit.date.year), (Relation::Before, 1582));
        let spec = parse("p=2067&template=Meteoryt&parameters=masa&unit=Q41803&decimalmark=,");
        assert_eq!(spec.unit, Some(ItemId(41_803)));
        assert_eq!(spec.decimal_mark, DecimalMark::Comma);
        assert_eq!(parse("unit=1").unit, None);
    }

    #[test]
    fn round_trip() {
        let mut spec = parse(
            "siteid=fr&project=wikisource&namespace=102&p=P50&template=Auteur&parameters=a|b&category=X&depth=3&alreadyset=0&wikisyntax=0&searchvalue=(\\d)&replacevalue=$1&constraints=Q5",
        );
        spec.manual_list = vec!["Page one".into(), "Q42".into()];
        spec.link_choice = LinkChoice::Last;
        spec.value_pattern = "{1}-{2}".into();
        spec.unwrap_templates = true;
        spec.lead_only = true;
        spec.archive_urls = ArchiveUrls::Skip;
        spec.transform.case = Case::Upper;
        spec.coordinate_parameters = Some(CoordinateParameters { latitude: "lat".into(), longitude: "long".into() });
        spec.qualifiers = vec![
            QualifierSpec { property: PropertyId(407), source: QualifierSource::Fixed { value: "Q1860".into() } },
            QualifierSpec {
                property: PropertyId(585),
                source: QualifierSource::Parameter { names: vec!["date".into(), "datum".into()] },
            },
        ];
        let pairs: Vec<(String, String)> = spec
            .to_legacy_query()
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), urlencoding::decode(v).unwrap().into_owned()))
            .collect();
        assert_eq!(JobSpec::from_legacy_query(&pairs), spec);
    }

    #[test]
    fn json_round_trip() {
        let spec = parse("p=569&template=X&aparameter1=year&aparameter2=month");
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(serde_json::from_str::<JobSpec>(&json).unwrap(), spec);
        assert_eq!(spec.date_parameters.unwrap().day, None);
    }
}
