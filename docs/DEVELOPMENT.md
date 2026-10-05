# Development and deployment

## Local setup

```sh
cp config.json.template config.json   # never commit config.json
cargo run -- --config config.json     # http://localhost:8000
```

In `config.json`, set `server.cookie_secure` to `false` and `oauth.callback_url`
to `http://localhost:8000/api/auth/callback` (with a matching OAuth consumer).

**Tool database.** Either tunnel to ToolsDB:

```sh
ssh -N -L 3308:tools.db.svc.wikimedia.cloud:3306 you@login.toolforge.org
```

or run a throwaway MariaDB and point `tool_db.url` at it:

```sh
docker run -d --rm -p 3399:3306 -e MARIADB_ALLOW_EMPTY_ROOT_PASSWORD=1 -e MARIADB_DATABASE=ht mariadb:11.3
# "url": "mysql://root@127.0.0.1:3399/ht"
```

The schema is created on startup.

**Replicas.** Without access, candidate pages come from the MediaWiki API
(slower). To use a replica, tunnel it and map it in `replicas.overrides`:

```sh
ssh -N -L 3310:enwiki.analytics.db.svc.wikimedia.cloud:3306 you@login.toolforge.org
# "overrides": {"enwiki": "127.0.0.1:3310"}
```

## Tests

```sh
cargo test                                  # all fast tests; DB tests need Docker
cargo test -- --ignored                     # live tests: real wikis, replicas via config.json; read-only
cargo clippy --all-targets -- -D warnings
cargo fmt
```

- Pure logic (wikitext, values, specs) has table-driven unit tests; GitHub
  issues are named in the cases they cover.
- HTTP is tested against `wiremock`; `test_support::world()` is a small mocked
  enwiki + Wikidata.
- Storage and workers run against MariaDB in a testcontainer (`test_support::test_store`).
- Live tests are `#[ignore]`d and never edit.

## Deployment (Toolforge)

Not deployed yet; this is the intended setup. The tool is built with the
Toolforge build service; `Procfile` starts the web server with
`$TOOL_DATA_DIR/config.json`.

```sh
toolforge build start https://github.com/magnusmanske/harvesttemplates
toolforge webservice buildservice start --mount=all
```

In the production `config.json`: `tool_db.url` on `tools.db.svc.wikimedia.cloud`,
`replicas.host_pattern` `{dbname}.analytics.db.svc.wikimedia.cloud`,
`server.session_dir` under `$TOOL_DATA_DIR`, `server.html_dir` `html`, and the
OAuth consumer with the tool's callback URL. Keep the file mode `600`.
