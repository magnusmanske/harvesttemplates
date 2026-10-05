# Development and deployment

## Configuration

Three files, all gitignored; keep them mode `600`:

| File | Content |
|---|---|
| `config.json` | Hosts, paths, limits. No secrets. Start from `config.json.template`. |
| `replica.my.cnf` | `[client]` `user` and `password` for ToolsDB and the replicas. Toolforge puts it in the tool's home. |
| `oauth.ini` | OAuth 2 client: `application_key`, `application_secret`, `callback_url`. |

The credential files are found relative to `config.json` (see `db_credentials`
and `oauth_file`). The tool database is `{user}__harvesttemplates`; it and its
tables are created on startup.

## Local setup

```sh
cp config.json.template config.json
cargo run -- --config config.json     # http://localhost:8000
```

In `config.json`, set `server.cookie_secure` to `false`, and reach ToolsDB
through a tunnel (`tool_db.host` `127.0.0.1`, `tool_db.port` `3308`):

```sh
ssh -N -L 3308:tools.db.svc.wikimedia.cloud:3306 you@login.toolforge.org
```

**Replicas.** Without access, candidate pages come from the MediaWiki API
(slower). To use a replica, tunnel it and map it in `replicas.overrides`:

```sh
ssh -N -L 3310:enwiki.analytics.db.svc.wikimedia.cloud:3306 you@login.toolforge.org
# "overrides": {"enwiki": "127.0.0.1:3310"}
```

**Logging in locally.** MediaWiki sends users back to the client's registered
callback URL, which for the production client is the live tool. To log in
locally, register a separate OAuth 2 client with callback
`http://localhost:8000/callback` and point `oauth_file` at its file
(e.g. `oauth.local.ini`; `oauth*.ini` is gitignored).

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

In `$TOOL_DATA_DIR`: `config.json` (the template's hosts are the production
ones; set `server.session_dir` to an absolute path there), `replica.my.cnf`
(provided by Toolforge) and `oauth.ini`.
