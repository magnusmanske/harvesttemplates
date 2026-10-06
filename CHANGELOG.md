# Changelog

User-visible changes. Numbers refer to [issues of the original tool](https://github.com/Pascalco/harvesttemplates/issues).
Add an entry under *Unreleased* with every change.

## Unreleased

## 0.3.0 – 2026-10-06

### Added
- The old tool's 621 shared queries are imported with their ids, so
  `index.html?htid=…` links work again and load like they used to.
- Values that someone removed from an item before are not added again (#89).
- Several properties from the same template in one run (#111).
- "Other wikis" on a run: the template's other language versions, each opening
  the same settings there (#206).

### Fixed
- Text that Wikibase's date parser rejects with an API error kept its year
  instead of failing the row.

## 0.2.0 – 2026-10-06

A rewrite of [Harvest Templates](https://pltools.toolforge.org/harvesttemplates/)
and its backend PLnode, at <https://harvesttemplates.toolforge.org/>. Old
permalinks keep working. Changes compared to the original:

### Reliability
- Harvests run on the server, not in the browser tab: closing the laptop no
  longer stops or skips anything (#140, #142). After a restart, runs are paused
  and can be resumed.
- Pages and category trees come from the database replicas, which fixes
  loading that hung or silently returned nothing (#208, #193); category walks
  have limits and finish with a clear message instead of hanging.
- Categories work with the current MediaWiki schema (`linktarget`).
- One unusable row no longer stops a run (#209, #207).
- "Already has a value" is checked on the live item right before each edit, so
  no more duplicates from a lagging query service (#211).
- Waits when Wikidata is lagged, and never sends an edit twice. Expired logins
  are renewed during long runs.

### Correctness
- Template parameters are read with a real parser: spaces around pipes (#204),
  redirects whose name starts like another template (#32), nested templates
  and links, `{{!}}` (#132), comments, references and `<nowiki>`.
- Dates: ISO dates keep their day (they were reduced to a year); years with
  fewer than four digits (#110); ranges and *c.*, *circa*, *vor*, *?* are
  rejected; leap years follow the calendar; calendar limits can be a full date
  (#135); Wikibase's own parser fills gaps in our month names (#56).
- Quantities: digits of other scripts, thousands separators, and units written
  after the number (#136); the unit must be allowed by the property
  (#156, #178).
- Items: links to the page itself are refused (#97); optionally the last link
  instead of the first (#149).
- Files: only files on Commons, not local ones with the same name (#104);
  encoded and oddly spaced names (#8, #49).
- Constraints are checked on the live item where possible; format patterns
  support PCRE features (#161); "distinct values" uses search, which still
  covers scholarly articles; the "instance or subclass of" relation works.
- A value of punctuation alone counts as no value (#147). An empty value no
  longer becomes the prefix that "add prefix" adds.

### New
- Preview: check every page before editing.
- Qualifiers on harvested statements, fixed or from another parameter
  (#210, #133).
- Coordinates, including `{{coord}}` inside infoboxes (#16).
- Combine parameters with a pattern like `{1}-{2}` (#52); use the content of
  nested templates such as `{{URL|x}}` (#2, #91); lower or upper case (#70);
  only templates before the first heading (#122); archive links become the
  original URL (#130).
- Filters: instances of given classes (#145), PetScan queries and SPARQL (#71);
  manual lists accept page titles and Q-ids (#109).
- Edit summaries say what was added (#175); every run is an edit group.
- Results link values to their target, e.g. external IDs via the formatter URL
  (#172); CSV log; the page title shows when a run is done (#112).
- Shared queries keep the full settings (#157), show the outcome of their last
  complete run (#137, #141), are searchable and sortable (#153), can be tagged
  (#174), and only their creator can delete them (#176). Saving no longer
  duplicates on reload (#138).
- The permalink stays current while editing the form (#115).
- Help and examples; works on phones.

### Security
- Login tokens stay on the server (they were passed in URLs).
- Database queries are parameterised (the old share page was open to SQL
  injection).
- Cross-site requests are refused; outbound requests only go to Wikimedia
  wikis; user-supplied regexes cannot exhaust the server.
