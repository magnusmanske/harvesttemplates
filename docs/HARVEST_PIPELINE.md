# Harvest pipeline

What happens to each candidate page (`harvest::pipeline::evaluate`), and every
message a user can see in the results table.

## Steps

1. **Item**: the page must be linked to a Wikidata item.
2. **Template**: the first transclusion of the template or an accepted redirect,
   in document order. Comments and `<ref>…</ref>` are ignored.
3. **Parameter**: the first listed parameter with a non-empty value. Unnamed
   parameters are `1`, `2`, … Alternatively the page title, or separate
   year/month/day parameters for dates.
4. **Clean**: nested templates removed, `[[target|label]]` → `[[target]]`,
   bold/italic and `&nbsp;` removed, whitespace collapsed, `{{!}}` handled.
5. **Transform**: add prefix/suffix, remove prefix/suffix (literal), then the
   regex search/replace (Rust `regex` syntax; `$1` works as in JavaScript).
6. **Parse** by datatype (below).
7. **Existing values**: on the live item. The exact value (or a more precise
   date) is always skipped; with "skip items with property set", any value is.
8. **Constraints**: the selected ones plus all mandatory ones. See `CONSTRAINTS.md`.
9. **Edit**: one `wbeditentity` with references *imported from* (P143, the
   wiki's item) and *Wikimedia import URL* (P4656, the exact revision).

## Datatypes

| Datatype | Rule |
|---|---|
| item | First (or last) `[[link]]`; with "plain links", the whole value as a title. Redirects are followed on the source wiki. |
| string, external-id | The value as is. |
| url | `[https://… label]` → URL; must be `http(s)://` without spaces. |
| commonsMedia | File name without namespace, `_` → space, percent-decoded. Must be on Commons, not only local. |
| time | See below. Calendar and optional date limit from the spec. |
| quantity | Any digit script, thousands separators removed, chosen decimal mark. Unit from the spec, must be allowed by the property. |
| monolingualtext | The value, with the language code from the spec. |

### Dates

Tried from most to least specific: day-month-year, month-day-year and
year-month-day with month names in the wiki's language (`data/monthnames.json`),
ISO `1950-05-12`, numeric `12.05.1950`/`1950/05/12` (Roman months allowed),
CJK `1950年5月12日`, month-year, and finally a bare year (1–4 digits alone, or
exactly one four-digit year in the text). Values with words like *circa*,
*vor*, *nach*, *años* or a `?` are rejected as imprecise. A date passes a limit
only if every day it could mean does.

To add a language, add its month names to `data/monthnames.json`.

## Messages

| Status | Message | Meaning |
|---|---|---|
| skipped | the page has no Wikidata item | |
| skipped | template not found | none of the accepted names on the page |
| skipped | no value | parameter missing or empty |
| skipped | the item already has this value | |
| skipped | the item already has the property | "skip items with property set" |
| error | could not find a date / imprecise date / ambiguous date: several years / invalid date | |
| error | date outside the configured range | date limit |
| error | unclear number | not a plain decimal number |
| error | no link to a target page / link to a section, not a page | |
| error | [[X]] does not exist / [[X]] has no Wikidata item | |
| error | the link points to the page itself | |
| error | not a file name / the file does not exist / the file is only on X, not on Commons | |
| error | not a URL | |
| error | the item does not exist | deleted item |
| error | constraint violation: *name* | |
| error | lookup failed: … | network problem; the row can be retried |
| error | *API message* | Wikidata refused the edit |

## Tips

- A template that sometimes includes a prefix (`tt0111161` vs `0111161`): use the
  regex `^(?:tt)?(\d+)$` → `tt$1` instead of "add prefix".
