//! Collecting a run's candidate pages, with the filters applied.

use super::job::Job;
use super::spec::SkipIf;
use crate::app_state::Clients;
use crate::ids::ItemId;
use crate::wiki::site::NS_CATEGORY;
use crate::wiki::{Limits, Page, PageSource};
use crate::wikitext::uppercase_first;
use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;

/// Pages that use the template but are not candidates, by reason.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Excluded {
    pub not_in_category: usize,
    pub not_in_list: usize,
    pub no_item: usize,
    /// Pre-filtered via WDQS; the live check before each edit catches the rest.
    pub already_set: usize,
}

pub async fn candidates(
    job: &Job,
    clients: &Clients,
    source: &dyn PageSource,
    limits: Limits,
) -> Result<(Vec<Page>, Excluded)> {
    let spec = &job.spec;
    let mut pages = source
        .transclusions(&job.site, &job.template_key, spec.namespace, limits)
        .await?;
    let mut excluded = Excluded::default();
    if !spec.category.trim().is_empty() {
        let category = job.site.db_key(NS_CATEGORY, &spec.category);
        let members = source
            .category_members(&job.site, &category, spec.namespace, spec.depth, limits)
            .await?;
        excluded.not_in_category = retain(&mut pages, |p| members.contains(&p.id));
    }
    if !spec.manual_list.is_empty() {
        let wanted = manual_list(&spec.manual_list);
        let listed = |p: &Page| wanted.contains(&p.title) || p.item.is_some_and(|q| wanted.contains(&q.to_string()));
        excluded.not_in_list = retain(&mut pages, listed);
    }
    excluded.no_item = retain(&mut pages, |p| p.item.is_some());
    if spec.skip_if == SkipIf::Property {
        let items: Vec<ItemId> = pages.iter().filter_map(|p| p.item).collect();
        match clients.wdqs.items_with_property(job.property.id, &items).await {
            Ok(set) => excluded.already_set = retain(&mut pages, |p| !p.item.is_some_and(|q| set.contains(&q))),
            Err(e) => tracing::warn!("WDQS pre-filter failed, relying on live checks: {e:#}"),
        }
    }
    Ok((pages, excluded))
}

/// Titles (any spacing, lower-case first letter) and item ids (#109).
fn manual_list(entries: &[String]) -> HashSet<String> {
    entries
        .iter()
        .map(|e| e.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|e| !e.is_empty())
        .map(|e| uppercase_first(&e))
        .collect()
}

/// Keep pages matching `keep`; return how many were dropped.
fn retain(pages: &mut Vec<Page>, keep: impl Fn(&Page) -> bool) -> usize {
    let before = pages.len();
    pages.retain(keep);
    before - pages.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_list_normalisation() {
        let list = manual_list(&["paris_ france ".into(), "q42".into(), "".into()]);
        assert!(list.contains("Paris france"));
        assert!(list.contains("Q42"));
        assert_eq!(list.len(), 2);
    }
}
