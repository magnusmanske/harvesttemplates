# Help

HarvestTemplates copies values from a template on a Wikimedia wiki into
Wikidata: it finds the pages that use the template, reads a parameter, turns it
into a Wikidata value and adds it to each page's item, with a reference to the
exact page revision. See [Examples](EXAMPLES.md) to get started.

## Workflow

1. **Fill in the form** (or open a permalink or a [shared query](https://harvesttemplates.toolforge.org/#/shares)).
2. **Load pages.** You need to log in. The tool collects the candidate pages
   and tells you how many were left out, and why.
3. **Preview** (recommended). Every page is checked, nothing is edited. Rows
   that would be added are marked *would add*.
4. **Start editing.** Edits are made in your name, about one per second. Each
   page is checked again right before its edit.
5. **Stop** whenever you like; **Start editing** continues where it stopped.

Runs happen on the server: you can close the browser and come back via
**My runs**. If the tool restarts, your run is paused; start it again.

## The form

**Load pages from**: the wiki and namespace.

**Define import**: the Wikidata property and the template. Redirects to the
template are listed; untick the ones that mean something else.

**Value**
- *Parameter*: its name, or `1`, `2`, … for unnamed parameters. Aliases are
  tried in order.
- *Combine parameters*: a pattern like `{1}-{2}`.
- *Use the page title instead*, e.g. for names.
- *Use the content of nested templates*: `{{URL|example.org}}` → `example.org`.
  Otherwise nested templates are ignored.
- Dates: one parameter, or separate year/month/day parameters; calendar; an
  optional limit such as *from 1926* (only dates certain in that calendar).
- Quantities: the unit, unless the value names it (`82 g`); the decimal mark.
- Coordinates: one parameter (text or a nested `{{coord}}`), or separate
  latitude/longitude parameters.
- Links to archived copies of web pages: use the original URL, skip, or keep.
- *Qualifiers*: added to every statement. Either a fixed value (`Q1860`,
  `2024-05-01`, `text@en`) or another parameter of the same template.
- *Also harvest*: more properties from the same template in the same run, e.g.
  date of death next to date of birth. They share the value options; the
  results have a row per page and property.

**Modify values**: add/remove a prefix or suffix, a regex search and replace
(`$1` for groups), lower or upper case.

**Filter**: a category (with depth), a list of page titles or Q-ids, a
PetScan query ID, a SPARQL query selecting `?item`, classes the items must be
instances of, templates before the first heading only, and whether to skip
items that already have *any* value for the property or only *this exact* one.
Values that someone removed from an item before are not added again, unless you
untick that option.

**Check constraints**: the property's constraints. Mandatory ones are always
checked; untick others you want to ignore.

## Results

| Status | Meaning |
|---|---|
| would add | Preview: this value would be added. |
| added | Added to Wikidata. |
| skipped | Nothing to do: no item, no template, no value, or the item already has it. |
| error | The value could not be used, broke a constraint, or Wikidata refused the edit. The message says which. |

**Download log** saves all rows as CSV. **Edit group** lists all edits of the
run, where they can be reviewed or undone. Every message is explained in
[the pipeline documentation](HARVEST_PIPELINE.md#messages).

## Sharing

**Permalink** (under the form) and **Edit as new run** (on a run) give a link
to the same settings. **Other wikis** (on a run) lists the template on other
language versions, each linking to the same settings there; check the parameter
names, as they often differ by language. **Share publicly** adds the query to the shared list,
with optional tags; `#/shares/TAG` lists one tag. The old tool's shared queries
are there too, and its links (`index.html?htid=…`) still work.

## Good to know

- Check a preview before editing large runs, and start with a small category.
- A run can include up to 500,000 pages; narrow it down with a category if needed.
- Dates with words like *circa*, *before* or a `?` are not imported.
- Problems and ideas: [GitHub issues](https://github.com/magnusmanske/harvesttemplates/issues).
