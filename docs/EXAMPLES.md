# Examples

Each link opens the form pre-filled. Click **Load pages**, then **Preview**, and
check some rows before you **Start editing**.

### IMDb IDs for upcoming films

English Wikipedia, `{{IMDb title}}` → [IMDb ID (P345)](https://www.wikidata.org/wiki/Property:P345),
in [Category:Upcoming films](https://en.wikipedia.org/wiki/Category:Upcoming_films) and its
subcategories, 30 levels deep. The regex adds `tt` where the template leaves it out.

[Open](https://harvesttemplates.toolforge.org/?siteid=en&project=wikipedia&namespace=0&p=P345&template=IMDb%20title&parameters=id%7C1&searchvalue=%5E%28%3F%3Att%29%3F%28%5Cd%2B%29%24&replacevalue=tt%241&category=Upcoming%20films&depth=30&alreadyset=1&wikisyntax=1)

### Spoken Wikipedia audio, with qualifiers

English Wikipedia, `{{Spoken Wikipedia}}` → [spoken text audio (P989)](https://www.wikidata.org/wiki/Property:P989),
with two qualifiers: language of work = English (fixed), and recording date
(from the template's `date` parameter).

[Open](https://harvesttemplates.toolforge.org/?siteid=en&project=wikipedia&namespace=0&p=P989&template=Spoken%20Wikipedia&parameters=1&alreadyset=1&wikisyntax=1&limityear=none&qualifier=P407%7Cfixed%7CQ1860&qualifier=P10135%7Cparam%7Cdate)

### Coordinates of lighthouses

English Wikipedia, `{{Infobox lighthouse}}` → [coordinate location (P625)](https://www.wikidata.org/wiki/Property:P625),
read from the `{{coord}}` inside the infobox. Only items that lack this exact
value are skipped, so you see which ones already match.

[Open](https://harvesttemplates.toolforge.org/?siteid=en&project=wikipedia&namespace=0&p=P625&template=Infobox%20lighthouse&parameters=coordinates&category=Lighthouses%20in%20Iceland&alreadyset=0&wikisyntax=1)

### Meteorite masses, with units

Polish Wikipedia, `{{Meteoryt infobox}}` → [mass (P2067)](https://www.wikidata.org/wiki/Property:P2067).
Values like `82 g` or `2,4 kg` carry their own unit; plain numbers get grams.
Decimal comma.

[Open](https://harvesttemplates.toolforge.org/?siteid=pl&project=wikipedia&namespace=0&p=P2067&template=Meteoryt%20infobox&parameters=masa&alreadyset=1&wikisyntax=1&unit=Q41803&decimalmark=%2C)

### Xeno-canto species IDs, from two parameters

English Wikipedia, `{{Xeno-canto species|Falco|columbarius}}` →
[Xeno-canto species ID (P2426)](https://www.wikidata.org/wiki/Property:P2426) `Falco-columbarius`,
with the pattern `{1}-{2}`.

[Open](https://harvesttemplates.toolforge.org/?siteid=en&project=wikipedia&namespace=0&p=P2426&template=Xeno-canto%20species&parameters=&alreadyset=1&wikisyntax=1&pattern=%7B1%7D-%7B2%7D)

### Links from the old tool

Permalinks of the old tool work here too: replace
`https://pltools.toolforge.org/harvesttemplates/` with
`https://harvesttemplates.toolforge.org/`. Links ending in `&run=` start loading
as soon as you are logged in; they never start editing on their own. Old
shared queries (`index.html?htid=…`) are available under their old ids.

More ready-made queries: [Shared queries](https://harvesttemplates.toolforge.org/#/shares).
