//! Page content and lookups the harvest pipeline needs from the source wiki.

use super::api::{MwApi, params};
use super::site::{NS_FILE, Site};
use crate::ids::ItemId;
use anyhow::Result;
use std::collections::HashMap;

const PAGES_PER_REQUEST: usize = 50;

#[derive(Debug, Clone)]
pub struct Revision {
    pub id: u64,
    pub text: String,
}

/// Current wikitext of pages, keyed by page id. Missing pages are absent.
pub async fn revisions(api: &MwApi, site: &Site, page_ids: &[u64]) -> Result<HashMap<u64, Revision>> {
    let mut out = HashMap::new();
    for chunk in page_ids.chunks(PAGES_PER_REQUEST) {
        let ids = chunk.iter().map(u64::to_string).collect::<Vec<_>>().join("|");
        let p = params(&[
            ("action", "query"),
            ("prop", "revisions"),
            ("rvprop", "ids|content"),
            ("rvslots", "main"),
            ("pageids", &ids),
        ]);
        let json = api.get(&site.host, &p).await?;
        for page in json["query"]["pages"].as_array().into_iter().flatten() {
            let rev = &page["revisions"][0];
            if let (Some(page_id), Some(id), Some(text)) = (
                page["pageid"].as_u64(),
                rev["revid"].as_u64(),
                rev["slots"]["main"]["content"].as_str(),
            ) {
                out.insert(
                    page_id,
                    Revision {
                        id,
                        text: text.to_string(),
                    },
                );
            }
        }
    }
    Ok(out)
}

/// Where a link on the source wiki points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Item(ItemId),
    NoItem,
    Missing,
}

/// Follows redirects, so `[[NYC]]` finds the item of New York City.
pub async fn link_target(api: &MwApi, site: &Site, title: &str) -> Result<LinkTarget> {
    let p = params(&[
        ("action", "query"),
        ("titles", title),
        ("redirects", "1"),
        ("prop", "pageprops"),
        ("ppprop", "wikibase_item"),
    ]);
    let json = api.get(&site.host, &p).await?;
    let page = &json["query"]["pages"][0];
    if page.is_null() || page.get("missing").is_some() || page.get("invalid").is_some() {
        return Ok(LinkTarget::Missing);
    }
    Ok(
        match page["pageprops"]["wikibase_item"].as_str().and_then(|q| q.parse().ok()) {
            Some(item) => LinkTarget::Item(item),
            None => LinkTarget::NoItem,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileLocation {
    Commons,
    /// Uploaded to the source wiki only; a Commons file of the same name would be a different file (#104).
    Local,
    Missing,
}

pub async fn file_location(api: &MwApi, site: &Site, name: &str) -> Result<FileLocation> {
    let title = site.full_title(NS_FILE, name);
    let p = params(&[
        ("action", "query"),
        ("titles", &title),
        ("prop", "imageinfo"),
        ("iiprop", ""),
    ]);
    let json = api.get(&site.host, &p).await?;
    let repository = json["query"]["pages"][0]["imagerepository"]
        .as_str()
        .unwrap_or_default();
    Ok(match repository {
        "shared" => FileLocation::Commons,
        "local" if site.host == "commons.wikimedia.org" => FileLocation::Commons,
        "local" => FileLocation::Local,
        _ => FileLocation::Missing,
    })
}
