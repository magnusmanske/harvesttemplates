# Architecture

One binary, one process. An axum server serves the JSON API under `/api`, the
static frontend from `html/`, and runs harvests as background tokio tasks.
State that must survive restarts lives in ToolsDB.

```
browser ──▶ api/ ──┬─▶ harvest/ (jobs, pipeline, workers)
                   │      │
                   │      ├─▶ wiki/       source wikis: API, replicas, site info
                   │      ├─▶ wikidata/   entities, WDQS, statements
                   │      ├─▶ constraints/
                   │      └─▶ auth/       OAuth, editing as the user
                   └─▶ storage/ (ToolsDB: runs, rows, shares)
```

## Modules

| Module | Responsibility |
|---|---|
| `api` | Routes, error mapping, sessions, CSRF guard. Thin: validates input, calls `harvest`/`storage`. |
| `harvest::spec` | `JobSpec`: what to harvest. JSON for the API and DB; converts to/from old permalinks. |
| `harvest::job` | `Job::prepare`: validates a spec against live data once per run. |
| `harvest::load` | Candidate pages: transclusions, category and list filters, WDQS pre-filter. |
| `harvest::pipeline` | One page → planned edit or a skip/error message. Pure apart from lookups. |
| `harvest::worker` | Background tasks: load a run; preview or edit its rows. |
| `harvest::active` | Which runs are being worked on; per-user limit; stop flags. |
| `wikitext` | Finding a template's parameters; cleaning values. No full parser. |
| `value` | Parsing values per datatype: dates (~250 languages), numbers, links, files, URLs. |
| `wiki` | Site metadata, MediaWiki API client, replica pools, page sources. |
| `wikidata` | Entity/property reads, WDQS, statement JSON. |
| `constraints` | The 23 supported constraint types, local-first. |
| `auth` | OAuth 2 login and token refresh, session store, `Editor` for `wbeditentity`. |
| `storage` | All SQL. Schema in `storage/schema.sql`, applied at startup. |

## A run's life

```
POST /api/runs ─▶ loading ─▶ ready ─┬─▶ previewing ─▶ ready
                     │              └─▶ editing ─▶ done
                     ▼                    │  ▲
                   failed              paused (stop, restart)
```

1. **Create**: the spec is validated (`Job::prepare`), a run row is written, and
   a task collects candidates into `run_row` (status `pending`). Pages left out
   are counted by reason in `run.excluded`.
2. **Preview** (optional): each pending row is evaluated; rows become `ready`,
   `skipped` or `error`. Nothing is edited.
3. **Start**: each `pending`/`ready` row is evaluated again against the current
   page revision and item, then edited. Rows become `done`, `skipped` or `error`.
4. **Stop** or a restart leaves the run `paused`; starting again continues.

OAuth tokens live only in the user's session file and in process memory;
nothing secret is stored in the database. Access tokens last a few hours, so
workers refresh them. MediaWiki rotates refresh tokens on use, which makes the
copy in the session stale: `auth::TokenCache` keeps the freshest token per user.
After a restart, a stale session means logging in again.

## Design decisions

- **Server-side runs**: closing the browser does not stop a run; progress is polled.
- **Replicas first**: candidates and category trees come from the wiki replicas
  (`templatelinks`/`categorylinks` via `linktarget`), with the API as fallback.
  Category walks are capped (depth, pages, categories) and never revisit a category.
- **Live checks before edits**: "already has a value" and most constraints are
  checked on the item fetched right before the edit, because WDQS lags.
- **No duplicate edits**: edit requests are never retried blindly; on `maxlag`
  the worker waits and re-sends the same edit, which the API rejected unsaved.
- **Hosts are built, not accepted**: outbound requests go only to hosts made
  from a validated language code and a fixed project list (`wiki::site::host_for`).
