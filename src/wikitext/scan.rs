//! Low-level brace/link-aware scanning shared by the template finder and the
//! value cleaner. All delimiters are ASCII, so byte offsets are valid `str`
//! slice boundaries.

use std::ops::Range;

/// A `{{…}}` transclusion: the byte ranges of its `|`-separated parts
/// (part 0 is the name) and the offset just past the closing `}}`.
#[derive(Debug)]
pub struct Call {
    pub parts: Vec<Range<usize>>,
    pub end: usize,
}

/// Parse the transclusion starting at `start` (which must point at `{{`).
/// Pipes inside nested templates, links and `<nowiki>` do not split parts.
pub fn scan_call(text: &str, start: usize) -> Option<Call> {
    let b = text.as_bytes();
    let mut i = start + 2;
    let (mut braces, mut links) = (0usize, 0usize);
    let mut part_start = i;
    let mut parts = Vec::new();
    while i < b.len() {
        if let Some(after) = skip_nowiki(text, i) {
            i = after;
            continue;
        }
        match &b[i..] {
            [b'{', b'{', ..] => braces += 1,
            [b'}', b'}', ..] if braces == 0 => {
                parts.push(part_start..i);
                return Some(Call { parts, end: i + 2 });
            }
            [b'}', b'}', ..] => braces -= 1,
            [b'[', b'[', ..] => links += 1,
            [b']', b']', ..] => links = links.saturating_sub(1),
            [b'|', ..] if braces == 0 && links == 0 => {
                parts.push(part_start..i);
                part_start = i + 1;
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    None
}

/// Position of the first `needle` byte in `text` that is outside nested
/// templates and links.
pub fn find_top_level(text: &str, needle: u8) -> Option<usize> {
    let b = text.as_bytes();
    let (mut braces, mut links) = (0usize, 0usize);
    let mut i = 0;
    while i < b.len() {
        match &b[i..] {
            [b'{', b'{', ..] => braces += 1,
            [b'}', b'}', ..] => braces = braces.saturating_sub(1),
            [b'[', b'[', ..] => links += 1,
            [b']', b']', ..] => links = links.saturating_sub(1),
            [c, ..] if *c == needle && braces == 0 && links == 0 => return Some(i),
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    None
}

/// If a `<nowiki>` block starts at `i`, return the offset just past it.
fn skip_nowiki(text: &str, i: usize) -> Option<usize> {
    const OPEN: &str = "<nowiki>";
    const CLOSE: &str = "</nowiki>";
    let rest = text.get(i..)?;
    if !rest.get(..OPEN.len())?.eq_ignore_ascii_case(OPEN) {
        return None;
    }
    let close = rest.to_ascii_lowercase().find(CLOSE);
    Some(close.map_or(text.len(), |c| i + c + CLOSE.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts<'a>(text: &'a str, call: &Call) -> Vec<&'a str> {
        call.parts.iter().map(|r| &text[r.clone()]).collect()
    }

    #[test]
    fn splits_top_level_pipes_only() {
        let t = "{{A|x=[[B|c]]|{{D|e}}|f}}";
        let call = scan_call(t, 0).unwrap();
        assert_eq!(parts(t, &call), ["A", "x=[[B|c]]", "{{D|e}}", "f"]);
        assert_eq!(call.end, t.len());
    }

    #[test]
    fn nowiki_is_opaque() {
        let t = "{{A|<nowiki>}}|</nowiki>|b}}";
        let call = scan_call(t, 0).unwrap();
        assert_eq!(parts(t, &call), ["A", "<nowiki>}}|</nowiki>", "b"]);
    }

    #[test]
    fn unbalanced_returns_none() {
        assert!(scan_call("{{A|b", 0).is_none());
    }

    #[test]
    fn top_level_equals() {
        assert_eq!(find_top_level("{{a|b=c}}", b'='), None);
        assert_eq!(find_top_level("k = {{a|b=c}}", b'='), Some(2));
    }
}
