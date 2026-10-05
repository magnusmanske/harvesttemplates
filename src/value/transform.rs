use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// User-defined edits applied to the cleaned value before parsing, in the
/// original tool's order: add affixes, remove affixes, regex replace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransformSpec {
    pub add_prefix: String,
    pub add_suffix: String,
    pub remove_prefix: String,
    pub remove_suffix: String,
    pub search: String,
    pub replace: String,
}

#[derive(Debug, Clone)]
pub struct Transform {
    spec: TransformSpec,
    search: Option<Regex>,
}

/// Generous for real patterns, small enough that a hostile one can't hog memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

impl Transform {
    pub fn new(spec: TransformSpec) -> Result<Self, regex::Error> {
        let search = match spec.search.as_str() {
            "" => None,
            pattern => Some(RegexBuilder::new(pattern).size_limit(REGEX_SIZE_LIMIT).build()?),
        };
        Ok(Self { spec, search })
    }

    pub fn apply(&self, value: &str) -> String {
        let s = &self.spec;
        let value = format!("{}{value}{}", s.add_prefix, s.add_suffix);
        let value = value.strip_prefix(s.remove_prefix.as_str()).unwrap_or(&value);
        let value = value.strip_suffix(s.remove_suffix.as_str()).unwrap_or(value);
        match &self.search {
            Some(re) => re.replace_all(value, js_replacement(&s.replace)).into_owned(),
            None => value.to_string(),
        }
    }
}

/// JavaScript reads `$1a` as group 1 followed by `a`; the regex crate would look
/// for a group named `1a`. Bracing the numbers keeps old permalinks working.
fn js_replacement(replacement: &str) -> String {
    let mut out = String::with_capacity(replacement.len());
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek().is_some_and(char::is_ascii_digit) {
            out.push_str("${");
            while let Some(d) = chars.next_if(char::is_ascii_digit) {
                out.push(d);
            }
            out.push('}');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(spec: TransformSpec, value: &str) -> String {
        Transform::new(spec).unwrap().apply(value)
    }

    #[test]
    fn affixes() {
        let spec = TransformSpec {
            add_prefix: "tt".into(),
            ..Default::default()
        };
        assert_eq!(transform(spec, "0111161"), "tt0111161");
        // #170: literal prefix including its trailing space
        let spec = TransformSpec {
            remove_prefix: "prefix ".into(),
            ..Default::default()
        };
        assert_eq!(transform(spec, "prefix [[value]]"), "[[value]]");
        let spec = TransformSpec {
            remove_suffix: ".html".into(),
            ..Default::default()
        };
        assert_eq!(transform(spec, "a.b.html"), "a.b");
    }

    #[test]
    fn regex_replace_with_js_groups() {
        let spec = TransformSpec {
            search: r"^(\d+)-(\d+)$".into(),
            replace: "$2a$1".into(),
            ..Default::default()
        };
        assert_eq!(transform(spec, "12-34"), "34a12");
    }

    #[test]
    fn invalid_regex_is_rejected() {
        let spec = TransformSpec {
            search: "(".into(),
            ..Default::default()
        };
        assert!(Transform::new(spec).is_err());
        let spec = TransformSpec {
            search: r"(a)\1".into(),
            ..Default::default()
        };
        assert!(Transform::new(spec).is_err(), "backreferences are not supported");
    }
}
