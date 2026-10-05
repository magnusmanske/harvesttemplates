# HarvestTemplates

Copy data from templates on Wikimedia projects into [Wikidata](https://www.wikidata.org).

Pick a wiki, a template, a parameter and a Wikidata property. HarvestTemplates
finds every page using the template, reads the parameter, turns it into a
Wikidata value, checks it against the property's constraints and adds it to
the page's item, with a reference to the exact page revision it came from.

This is a rewrite in Rust of [Pasleim's Harvest Templates](https://github.com/Pascalco/harvesttemplates)
(and its backend [PLnode](https://github.com/Pascalco/PLnode)), which has been
unmaintained since 2022. It keeps the interface and the permalink format, and fixes
the long-standing problems:

- Harvests run **on the server**, not in your browser tab. Close the laptop, come back later.
- Large templates and deep category trees load from the database replicas instead of hanging.
- "Already has a value" is checked against the live item right before each edit, so no duplicates.
- A proper template parser handles spaces, nested templates, `{{!}}` and comments.
- One bad row is logged and skipped; it no longer stops the run.

> **Status:** phases 1 and 2 of [the plan](PLAN.md) are done: everything the
> original did, plus qualifiers, coordinates, units, PetScan/SPARQL filters and
> more. Loading and preview are verified on live wikis; editing is tested
> against mocks only so far. Not deployed yet.

## Documentation

- [Architecture](docs/ARCHITECTURE.md): modules, the life of a run, design decisions
- [Harvest pipeline](docs/HARVEST_PIPELINE.md): what happens to each page, value parsing, every message
- [Constraint checks](docs/CONSTRAINTS.md)
- [HTTP API](docs/API.md)
- [Development and deployment](docs/DEVELOPMENT.md)

## Quick start

```sh
cp config.json.template config.json   # hosts and paths; no secrets
# put replica.my.cnf and oauth.ini next to it; never commit any of the three
cargo run -- --config config.json     # http://localhost:8000
cargo test                            # DB tests need Docker
```

Code style and project rules are in [CLAUDE.md](CLAUDE.md).

## License

[MIT](LICENSE). The original tool by Pasleim (and PLnode, whose month-name data is reused here) is CC0.
