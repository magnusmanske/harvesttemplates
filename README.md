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

> **Status:** under construction. See [PLAN.md](PLAN.md) for the roadmap.

## Running locally

Requirements: Rust (stable), and for the database tests Docker.

```sh
cp config.json.template config.json   # fill in credentials; never commit this file
cargo run -- --config config.json     # http://localhost:8000
```

The tool database and wiki replicas live on Toolforge. Reach them through SSH tunnels:

```sh
ssh -N -L 3308:tools.db.svc.wikimedia.cloud:3306 you@login.toolforge.org
ssh -N -L 3310:enwiki.analytics.db.svc.wikimedia.cloud:3306 you@login.toolforge.org
```

and map them in `config.json`: point `tool_db.url` at `127.0.0.1:3308`, and add
`"enwiki": "127.0.0.1:3310"` to `replicas.overrides`. Wikis without a tunnel
fall back to the (slower) MediaWiki API.

## Development

```sh
cargo test                                # unit + DB tests (needs Docker)
cargo clippy --all-targets -- -D warnings
cargo fmt
```

CI runs all three on every push. Code style and project rules are in [CLAUDE.md](CLAUDE.md).

## License

[CC0 1.0](LICENSE), like the original. Original tool by Pasleim; rewrite by Magnus Manske.
