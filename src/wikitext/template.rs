use super::scan::{find_top_level, scan_call};
use super::uppercase_first;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

static COMMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?(?:-->|\z)").unwrap());
static REFS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<ref(?:\s[^>]*)?/>|<ref(?:\s[^>]*)?>.*?</ref\s*>").unwrap());

/// Recognises transclusions of one template under any of its names.
#[derive(Debug, Clone)]
pub struct TemplateMatcher {
    names: HashSet<String>,
    /// Lower-cased template namespace names and aliases, e.g. `template`, `vorlage`.
    namespace_prefixes: Vec<String>,
    first_letter_case_insensitive: bool,
}

impl TemplateMatcher {
    /// `names` are the template and the redirects to it, without namespace prefix.
    /// `first_letter_case_insensitive` is `false` only on wikis like Wiktionary.
    pub fn new(
        names: impl IntoIterator<Item = impl AsRef<str>>,
        namespace_prefixes: impl IntoIterator<Item = impl AsRef<str>>,
        first_letter_case_insensitive: bool,
    ) -> Self {
        let mut matcher = Self {
            names: HashSet::new(),
            namespace_prefixes: namespace_prefixes
                .into_iter()
                .map(|p| normalize_spaces(p.as_ref()).to_lowercase())
                .collect(),
            first_letter_case_insensitive,
        };
        matcher.names = names.into_iter().map(|n| matcher.normalize(n.as_ref())).collect();
        matcher
    }

    /// Parameters of the first matching transclusion in `wikitext`, in document
    /// order (an outer template comes before the templates nested in it).
    /// Comments and `<ref>`s are ignored, as their content rarely describes the page subject.
    pub fn find(&self, wikitext: &str) -> Option<TemplateParams> {
        let text = COMMENTS.replace_all(wikitext, "");
        let text = REFS.replace_all(&text, "");
        self.find_in(&text)
    }

    fn find_in(&self, text: &str) -> Option<TemplateParams> {
        let mut pos = 0;
        while let Some(offset) = text[pos..].find("{{") {
            let start = pos + offset;
            let Some(call) = scan_call(text, start) else {
                pos = start + 2;
                continue;
            };
            let parts: Vec<&str> = call.parts.iter().map(|r| &text[r.clone()]).collect();
            if self.names.contains(&self.normalize(parts[0])) {
                return Some(TemplateParams::from_parts(&parts[1..]));
            }
            if let Some(found) = parts.iter().find_map(|part| self.find_in(part)) {
                return Some(found);
            }
            pos = call.end;
        }
        None
    }

    fn normalize(&self, name: &str) -> String {
        let name = normalize_spaces(name);
        let name = name.strip_prefix(':').unwrap_or(&name).trim_start();
        let name = self.strip_namespace(name);
        if self.first_letter_case_insensitive {
            uppercase_first(name)
        } else {
            name.to_string()
        }
    }

    fn strip_namespace<'a>(&self, name: &'a str) -> &'a str {
        let Some((prefix, rest)) = name.split_once(':') else {
            return name;
        };
        let prefix = prefix.trim_end().to_lowercase();
        if self.namespace_prefixes.contains(&prefix) {
            rest.trim_start()
        } else {
            name
        }
    }
}

/// Parameters of one transclusion. Unnamed parameters are keyed `"1"`, `"2"`, …
/// Values are trimmed but otherwise raw; see [`super::clean_value`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateParams(HashMap<String, String>);

impl TemplateParams {
    fn from_parts(parts: &[&str]) -> Self {
        let mut params = HashMap::new();
        let mut unnamed = 0;
        for part in parts {
            let (key, value) = match find_top_level(part, b'=') {
                Some(eq) => (part[..eq].trim().to_string(), &part[eq + 1..]),
                None => {
                    unnamed += 1;
                    (unnamed.to_string(), *part)
                }
            };
            // Later duplicates win, as in MediaWiki.
            params.insert(key, value.trim().to_string());
        }
        Self(params)
    }

    /// The value of `name`, if present and non-empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name.trim()).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// The value of the first of `names` that is present and non-empty.
    pub fn first_of<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<&str> {
        names.into_iter().find_map(|n| self.get(n))
    }
}

fn normalize_spaces(s: &str) -> String {
    s.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(names: &[&str]) -> TemplateMatcher {
        TemplateMatcher::new(names.iter().copied(), ["Template", "Vorlage"], true)
    }

    fn get(names: &[&str], text: &str, param: &str) -> Option<String> {
        matcher(names).find(text)?.get(param).map(str::to_string)
    }

    #[test]
    fn named_and_unnamed() {
        let t = "{{IMDb title|0111161|The Shawshank Redemption|id2=x}}";
        assert_eq!(get(&["IMDb title"], t, "1").as_deref(), Some("0111161"));
        assert_eq!(
            get(&["IMDb title"], t, "2").as_deref(),
            Some("The Shawshank Redemption")
        );
        assert_eq!(get(&["IMDb title"], t, "id2").as_deref(), Some("x"));
    }

    #[test]
    fn issue_204_spaces_around_pipes() {
        let t = "{{RömppOnline |ID=RD-18-01251 |Name=Rhodamine |Abruf=2016-08-08}}";
        assert_eq!(get(&["RömppOnline"], t, "ID").as_deref(), Some("RD-18-01251"));
    }

    #[test]
    fn issue_32_redirect_is_not_a_prefix_match() {
        let t = "{{BDCL|wrong}} {{BD|1950|1999}}";
        assert_eq!(get(&["NF", "BD"], t, "1").as_deref(), Some("1950"));
    }

    #[test]
    fn multiline_infobox_with_nested_templates_and_links() {
        let t = "Text\n{{Infobox person\n | name = {{lang|fr|Jean}}\n | birth_place = [[Paris|the city]]\n | url = http://x.org/?a=b\n}}";
        assert_eq!(
            get(&["Infobox person"], t, "birth_place").as_deref(),
            Some("[[Paris|the city]]")
        );
        assert_eq!(get(&["Infobox person"], t, "name").as_deref(), Some("{{lang|fr|Jean}}"));
        assert_eq!(get(&["Infobox person"], t, "url").as_deref(), Some("http://x.org/?a=b"));
    }

    #[test]
    fn equals_inside_nested_template_is_unnamed() {
        let t = "{{X|{{Y|a=b}}}}";
        assert_eq!(get(&["X"], t, "1").as_deref(), Some("{{Y|a=b}}"));
    }

    #[test]
    fn name_normalisation() {
        assert!(get(&["Infobox person"], "{{infobox_person|a=1}}", "a").is_some());
        assert!(get(&["Infobox person"], "{{ Template : Infobox  person |a=1}}", "a").is_some());
        assert!(get(&["Normdaten"], "{{Vorlage:Normdaten|GND=1}}", "GND").is_some());
        assert!(get(&["Normdaten"], "{{Other:Normdaten|GND=1}}", "GND").is_none());
    }

    #[test]
    fn case_sensitive_wikis() {
        let m = TemplateMatcher::new(["en-noun"], ["Template"], false);
        assert!(m.find("{{En-noun|x}}").is_none());
        assert!(m.find("{{en-noun|x}}").is_some());
    }

    #[test]
    fn nested_target_is_found() {
        let t = "{{Infobox company|homepage = {{URL|brenntag.com}}}}";
        assert_eq!(get(&["URL"], t, "1").as_deref(), Some("brenntag.com"));
    }

    #[test]
    fn comments_and_refs_are_ignored() {
        let t = "<!-- {{X|1=no}} -->{{X|id=<ref>{{cite|id=no}}</ref> 42 <ref name=\"a\"/>}}";
        assert_eq!(get(&["X"], t, "id").as_deref(), Some("42"));
        assert!(get(&["X"], "<!-- {{X|1=no}}", "1").is_none());
    }

    #[test]
    fn empty_values_count_as_missing() {
        let p = matcher(&["X"]).find("{{X|a=|b=2|a= }}").unwrap();
        assert_eq!(p.get("a"), None);
        assert_eq!(p.first_of(["a", "b"]), Some("2"));
    }

    #[test]
    fn later_duplicates_win() {
        assert_eq!(get(&["X"], "{{X|a=1|a=2}}", "a").as_deref(), Some("2"));
    }

    #[test]
    fn unbalanced_text_does_not_panic() {
        assert!(get(&["X"], "{{X|a=1", "a").is_none());
        assert_eq!(get(&["X"], "}} {{ {{X|a=ü}}", "a").as_deref(), Some("ü"));
    }
}
