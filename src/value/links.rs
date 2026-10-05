//! Pull link targets, file names and URLs out of cleaned wikitext values.

use super::ValueError;
use regex::Regex;
use std::sync::LazyLock;

static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^|\]]+)").unwrap());
static EXTERNAL_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\s\]]+)(?:\s[^\]]*)?\]").unwrap());

/// Which link to use when a value contains several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkChoice {
    #[default]
    First,
    /// The most specific place is often last: `[[Nagano]][[Suwa]][[Fujimi]]` (#149).
    Last,
}

/// Title of the linked page. Without a wikilink, the whole value is used
/// only if `allow_plain` is set ("match target page even without wikisyntax").
pub fn link_target(value: &str, allow_plain: bool, choice: LinkChoice) -> Result<String, ValueError> {
    let mut links = WIKILINK.captures_iter(value).map(|c| c[1].trim().to_string());
    let target = match choice {
        LinkChoice::First => links.next(),
        LinkChoice::Last => links.last(),
    };
    let target = match target {
        Some(t) => t,
        None if allow_plain && !value.trim().is_empty() => value.trim().to_string(),
        None => return Err(ValueError::NoLink),
    };
    if target.contains('#') {
        return Err(ValueError::SectionLink);
    }
    Ok(target)
}

/// File name without namespace prefix, as Wikidata stores it: `Foo bar.jpg`.
/// `prefixes` are the wiki's File namespace names and aliases.
pub fn file_name(value: &str, prefixes: &[String]) -> Result<String, ValueError> {
    let name = WIKILINK
        .captures(value)
        .map_or(value, |c| c.get(1).map_or(value, |m| m.as_str()));
    let name = strip_namespace(name.trim(), prefixes);
    let name = urlencoding::decode(name).map_or_else(|_| name.to_string(), |d| d.into_owned());
    let name = name.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || !name.contains('.') {
        return Err(ValueError::NotAFile);
    }
    Ok(uppercase_first(&name))
}

/// The URL of `[https://example.org label]`, or the value itself.
pub fn url(value: &str) -> Result<String, ValueError> {
    let url = EXTERNAL_LINK
        .captures(value)
        .and_then(|c| c.get(1))
        .map_or(value, |m| m.as_str())
        .trim();
    let is_web = url.starts_with("https://") || url.starts_with("http://");
    if !is_web || url.contains(char::is_whitespace) {
        return Err(ValueError::NotAUrl);
    }
    Ok(url.to_string())
}

fn strip_namespace<'a>(name: &'a str, prefixes: &[String]) -> &'a str {
    let Some((prefix, rest)) = name.split_once(':') else {
        return name;
    };
    let prefix = prefix.trim().replace('_', " ").to_lowercase();
    if prefixes.iter().any(|p| p.to_lowercase() == prefix) {
        rest.trim_start()
    } else {
        name
    }
}

fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_targets() {
        assert_eq!(
            link_target("[[Paris]]", false, LinkChoice::First).as_deref(),
            Ok("Paris")
        );
        assert_eq!(
            link_target("born in [[Paris]], [[France]]", false, LinkChoice::Last).as_deref(),
            Ok("France")
        );
        assert_eq!(link_target("Paris", true, LinkChoice::First).as_deref(), Ok("Paris"));
        assert_eq!(link_target("Paris", false, LinkChoice::First), Err(ValueError::NoLink));
        assert_eq!(
            link_target("[[Paris#History]]", false, LinkChoice::First),
            Err(ValueError::SectionLink)
        );
    }

    #[test]
    fn file_names() {
        let prefixes = vec!["File".to_string(), "Datei".to_string(), "Image".to_string()];
        let cases = [
            ("[[File:Foo_bar.jpg]]", "Foo bar.jpg"),
            ("Datei:Foo.jpg", "Foo.jpg"),
            ("foo.jpg", "Foo.jpg"),
            (
                "Carduelis_spinus_2_tom_%28Marek_Szczepanek%29.jpg",
                "Carduelis spinus 2 tom (Marek Szczepanek).jpg",
            ), // #8
            ("Falta  _ imagen.svg", "Falta imagen.svg"), // #49
            ("[[Image:A.png]]<br/>caption", "A.png"),
        ];
        for (raw, expected) in cases {
            assert_eq!(file_name(raw, &prefixes).as_deref(), Ok(expected), "{raw}");
        }
        assert_eq!(file_name("no file", &prefixes), Err(ValueError::NotAFile));
    }

    #[test]
    fn urls() {
        assert_eq!(
            url("[https://example.org/ Example]").as_deref(),
            Ok("https://example.org/")
        );
        assert_eq!(url("http://example.org").as_deref(), Ok("http://example.org"));
        assert_eq!(url("example.org"), Err(ValueError::NotAUrl));
    }
}
