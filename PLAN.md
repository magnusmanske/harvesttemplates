# HarvestTemplates (Rust) — implementation plan

A Rust rewrite of Pasleim's **Harvest Templates** tool: read a template parameter
from pages of a Wikimedia project and add it as a statement (with an import
reference) to the linked Wikidata item.

This document is the plan agreed before writing code. Once implemented, the
durable parts move into `docs/` and this file is deleted.

---

## 1. What the original is (findings)

The live tool is two Toolforge tools and three codebases:

| Part | Where | Tech | Role |
|---|---|---|---|
| Frontend | `pltools` → [Pascalco/harvesttemplates](https://github.com/Pascalco/harvesttemplates) | React 16 / CRA 3.4 (2020) | Form, candidate table, drives the harvest from the browser |
| Share page | same repo, `public/share.php`, `gethtshare.php` | PHP + jQuery | Public list of saved queries (`ht_share` table in ToolsDB) |
| Backend | `plnode` → [Pascalco/PLnode](https://github.com/Pascalco/PLnode) | Node/Express | OAuth, per-candidate harvest, constraint checks, category scan, edit |

Status: unmaintained since 2022. MisterSynergy is the de‑facto owner and has said he
would **redirect the current URL to a stable successor**. Eran (Eranroz) was added as
co‑maintainer in July 2026 and merged the `linktarget` category fix (PLnode PR #13),
but large category scans still hang. A Community Wishlist entry
([W373](https://meta.wikimedia.org/wiki/Community_Wishlist/W373)) asks for continued
development; its top asks are **qualifiers** and **not aborting on bad rows**.

### 1.1 How a harvest works today

1. Browser loads: siteinfo (namespaces, dbname), edition item (`P1800` via WDQS),
   all pages transcluding the template (API `generator=transcludedin`, paginated
   from the browser), template redirects, property info + constraints, and — if
   "do not load items with property set" — **every item with the property**
   (`SELECT ?item { ?item wdt:P345 [] }`). Category filter via backend replica SQL.
2. Browser intersects the lists, sorts by last edit, renders the table.
3. Browser POSTs candidates **one at a time** every 2 s to `/harvester`, which:
   fetches wikitext → regex‑extracts the template → picks the parameter →
   applies prefix/suffix/regex → parses by datatype → builds a statement with
   `P143` (edition item) + `P4656` (revision permalink) references →
   `/cc` constraint check (23 checkers, mostly WDQS `ASK`) → `/exists` →
   `wbeditentity` with the user's OAuth 1.0a token, `maxlag=10`, summary
   `([[:toollabs:editgroups/b/harvesttemplates/<id>|details]])`.

### 1.2 Why it breaks (root causes behind the hang reports)

- **Giant client‑side lists.** The "items with property" SPARQL is unbounded
  (millions of rows for P345). #208, #193, talk‑page reports.
- **Category scan** used the pre‑`linktarget` schema (fixed in PR #13) and has no
  depth/size cap — "Upcoming films, depth 30" still hangs.
- **WDQS lag** makes the "already set" pre‑filter stale → duplicates (#211).
- **Browser‑driven execution**: closing the laptop stops the run, reconnect skips
  an item (#142), no server‑side batches (#140).
- **Regex template parsing**: fails on spaces (#204), nested templates (#2, #91),
  `{{!}}` (#132), redirect name collisions (#32).
- **Rotting toolchain**: CRA 3 + 2020 npm deps, Node backend with string‑built
  SQL (`share.php` and `util.mjs` are SQL‑injectable), JWT passed in URLs.

### 1.3 Supported today (must keep)

Datatypes: `wikibase-item`, `string`, `external-id`, `url`, `commonsMedia`,
`time` (single param or year/month/day params, Gregorian/Julian, year cut‑off),
`quantity` (unit from allowed‑units constraint, decimal mark), `monolingualtext`
(or page title as value). Value transforms: add/remove prefix/suffix, regex
search/replace. Filters: namespace, category + depth, manual list (titles or
QIDs), skip items with property set. Template redirect selection. Per‑constraint
opt‑out (mandatory ones forced). Permalink and `?run=` autoplay, saved public
queries (`?htid=`), CSV log download, EditGroups integration. Month names for
227 languages and 14 numeral systems.

---

## 2. Design decisions

### D1 — One tool, one binary, one process
Frontend, API and harvest execution live in a single axum binary deployed with
the Toolforge build service (`Procfile: web: …`). No separate job runner for now:
harvests are I/O‑bound and edit‑rate‑limited, so an in‑process worker pool is
enough. Runs are persisted in ToolsDB so a restart loses nothing. If load ever
demands it, the worker loop can be moved to a `toolforge jobs` process without
changing the data model.

### D2 — The server does all the heavy lifting; the browser only views
`POST /api/runs` with the job spec creates a **run**. The server loads candidates
(replica SQL, batched), pre‑filters, then — on `start` — processes them. The
browser polls `GET /api/runs/{id}` (1–2 s) for progress and row results. This
fixes #140, #142, #208, #193, #209 and the talk‑page hangs by construction.
Polling over SSE: survives reconnects and proxies, trivially simple.

### D3 — Candidates come from the replicas, not the API
`templatelinks ⋈ linktarget ⋈ page ⟕ page_props(wikibase_item)` in one query per
wiki; category trees via `categorylinks ⋈ linktarget` with a depth cap **and** a
visited set **and** a hard row cap. A `PageSource` trait with a `Replica`
implementation and an `Api` fallback (local dev without tunnels; replica outages).

### D4 — "Already set" is checked live, at harvest time
Drop the unbounded SPARQL. For display, pre‑filter with batched
`VALUES ?item {…}` queries (bounded by candidate count, not property size).
The authoritative check is `wbgetentities` of the item immediately before the
edit — which we fetch anyway for local constraint checks. Fixes #211. Add the
option "skip only if this exact value exists" (#40).

### D5 — Real template parsing
A small balanced‑brace scanner (comments, `<nowiki>`, `<ref>`, nested
templates, `{{!}}`, named/unnamed params, redirect names) with table‑driven
tests, instead of regexes. Evaluate `parse_wiki_text_2` first; use it only if
it is maintained and handles our cases, otherwise ~200 lines of our own.

### D6 — Constraints: local first, WDQS only when unavoidable
Port all 23 checkers behind one `ConstraintCheck` trait and a registry table.
Checks that only need the target item (single value, conflicts‑with,
item‑requires‑statement, allowed units, format, range, one/none‑of, scope,
entity type, integer, no bounds, citation needed, mandatory/allowed qualifiers)
run on the already‑fetched entity. Only distinct, type, value‑type, inverse,
symmetric and value‑requires‑statement go to WDQS, with a per‑run cache.
Property constraint definitions are cached (moka, 10 min).

### D7 — Edits: user's own OAuth 2 tokens, direct `wbeditentity`
Same as the original and as `mixnmatch_rs`: one edit per statement with
references, `maxlag=5`, exponential back‑off on `maxlag`/429, summary keeps the
EditGroups pattern **and** says what was added (#175):
`Added [[Property:P345]]: tt0111161 from enwiki ([[:toollabs:editgroups/b/harvesttemplates/<id>|details]])`.
Tokens stay server‑side: in the session (login) and in the memory of a running
worker. Never in the database, URLs, or cookies readable by JS.

### D8 — Frontend without a build step
Vue 3 + Bootstrap 5 from `tools-static.wmflabs.org/cdnjs`, plain ES modules,
one `index.html`, a handful of component files. This is what keeps
`mixnmatch_rs` maintainable and avoids the dependency rot that killed the CRA
frontend.  All CSS in `html/main.css`.
URL parameter names stay **backward compatible** (`siteid, project, namespace,
p, template, templateredirects, parameters, …, htid, run`) — many permalinks
live on wiki pages. `share.php`, `index.html?htid=` are redirected.

### D9 — Typed config, secrets outside the repo
`config.json` (gitignored, no secrets) deserialised into a `Config` struct
(improvement over mixnmatch's untyped `Value`). Credentials stay in the files
Toolforge uses: `replica.my.cnf` (DB) and `oauth.ini` (OAuth 2 client).
Replica hosts derived from the wiki dbname.

### D10 — Security baseline
OAuth‑gated mutations; run ownership checks; CSRF via `SameSite` cookie +
`Origin` allow‑list; parameterised SQL only; user regexes compiled with the
`regex` crate (linear time, size‑limited); format constraints via `fancy_regex`
with a backtrack limit; wiki hostnames validated against a strict pattern and
the sitematrix; outbound HTTP via a single client with UA, timeouts and SSRF
guard; per‑user cap on concurrent runs; per‑IP rate limit on POST.

---

## 3. Architecture

```
browser ──HTTP──▶ axum ──┬─▶ api/        JSON API (runs, shares, auth, meta)
                         ├─▶ html/       static frontend (Vue 3, no build)
                         └─▶ harvest/    run workers (tokio tasks)
                                  │
            ┌─────────────────────┼─────────────────────┐
            ▼                     ▼                     ▼
      wiki/ (replicas,       wikidata/ (API, WDQS,   storage/ (ToolsDB:
       MW API per site)       edit, entity cache)     runs, items, shares)
```

### 3.1 Module layout (small files, one responsibility each)

```
src/
  main.rs            clap CLI: `webserver [--config] [--port] [--dev-auth]`, `migrate`
  lib.rs             lint policy (#![forbid(unsafe_code)], clippy warn set as in mixnmatch_rs)
  config.rs          typed Config + is_on_toolforge()
  app_state.rs       AppState { config, http, storage, replicas, caches, runs }
  api/
    router.rs        routes + middleware stack (trace, sessions, CORS, timeout, panic recovery, rate limit)
    error.rs         ApiError (BadRequest/Unauthorized/Forbidden/NotFound/Internal) → HTTP
    auth.rs          /api/auth/{login,callback,logout,me}
    runs.rs          POST /api/runs, GET /api/runs/{id}, POST …/start|stop, GET …/log.csv
    shares.rs        GET/POST/DELETE /api/shares, tags
    meta.rs          /api/site/{host} (namespaces, edition item), /api/property/{P} (datatype, constraints, units), template redirects
  auth/              OAuth 2 flow + token refresh, tower-sessions file store, require_user()
  wiki/
    site.rs          Site { host, dbname, lang, ns names/aliases, edition_qid } + validation
    replica.rs       lazy per-dbname mysql_async pool registry (host derived from dbname)
    page_source.rs   trait PageSource { transclusions(), category_tree() } + Replica/Api impls
    api_client.rs    MW API wrapper: wikitext (50 pages/call), siteinfo, redirects, pageprops
  harvest/
    spec.rs          JobSpec (serde; the permalink/share format) + validation
    run.rs           Run state machine: Created → Loading → Ready → Running ↔ Paused → Done/Failed
    worker.rs        per-run task: pacing, stop flag, persistence, resume
    pipeline.rs      per-candidate steps: wikitext → extract → transform → parse → check → edit
    candidate.rs     Candidate { page_id, title, qid, raw, parsed, status, message }
    summary.rs       edit summary + EditGroups id
  wikitext/
    template.rs      balanced-brace template extraction, params map, redirect names
    links.rs         link simplification, {{!}}, file/template prefixes
  value/
    mod.rs           ParsedValue enum → Wikibase datavalue JSON
    transform.rs     prefix/suffix/regex/case/split (#70, #147, #52)
    time.rs          date parser (month names, numerals, formats, calendar, limits)
    quantity.rs      number + unit suffix parsing (#136), decimal marks
    item.rs          link → QID resolution, first/last link (#149), self-link guard
    media.rs         file name normalisation + Commons/local existence (#104)
    url.rs           [url text] extraction, archive-URL handling (#130)
    coordinate.rs    lat/lon and dms params (#16)
    monthnames.rs    include_str!("../data/monthnames.json"), numerals table
  constraints/
    mod.rs           trait ConstraintCheck + REGISTRY table (QID → checker) + registry test
    propdata.rs      cached P2302 definitions per property
    <one file per constraint>
  wikidata/
    entity.rs        wbgetentities + per-run cache
    statement.rs     claim JSON builder (mainsnak, qualifiers, references)
    edit.rs          wbeditentity with CSRF token, maxlag, back-off, error mapping
    wdqs.rs          ASK + batched VALUES helpers, throttle
  storage/
    mod.rs           trait Storage (runs, run_items, shares, tags)
    mysql.rs         ToolsDB implementation
    schema.sql       tables (below)
  util/              http.rs (client factory, UA, SSRF guard), throttle.rs, ids.rs (newtypes)
html/                index.html, app.js, components/*.js, main.css, share.html
data/                monthnames.json, numerals.json (ported from PLnode)
docs/                see §7
```

### 3.2 Data model (ToolsDB)

```
run        id, user_central_id, user_name, spec JSON, status, editgroup, counts (total/ok/err/skip),
           created, started, finished, error
run_item   run_id, page_id, title, qid, revid, raw_value, parsed_value, status, message, edited_at
share      id, user_name, spec JSON, title, created, last_completed_run, runs_ok, runs_err
share_tag  share_id, tag
```
`run_item` gives resume, CSV log, and red/green counts on the share page (#141).
Shares store the full spec, so aliases are no longer lost (#157). Migration of
the existing `ht_share` rows needs a dump from the `pltools` maintainers.

### 3.3 Run lifecycle

```
POST /api/runs            validate spec → load site/property/constraints
                          → PageSource → pre-filter (category, manual list,
                          batched VALUES "has property") → Ready, rows persisted
POST /api/runs/{id}/start requires login; spawns worker (max N per user)
worker                    for each pending row: pipeline → persist row → pace (≈1 edit/s, maxlag back-off)
POST /api/runs/{id}/stop  sets flag; worker finishes current row
GET  /api/runs/{id}       status, counts, rows (paged), ETA
restart                   Running runs become Paused; the owner resumes them
```
A **preview** flag runs the pipeline without the edit step (the old "demo" mode),
returning parsed values and constraint results — valuable for checking before
committing (#172).

### 3.4 Per-candidate pipeline

```
wikitext (batched)     → template params  → raw value     → transformed
→ ParsedValue          → statement JSON   → local checks  → remote checks
→ exists? (live)       → wbeditentity     → row result
```
Each step returns `Result<_, Skip>` where `Skip` carries the user-facing
message; a failing row never aborts the run (#209, #207).

---

## 4. Issue triage

Every open issue mapped to a phase. "Design" = solved by the architecture.

| # | Issue | Phase | Note |
|---|---|---|---|
| 208, 193, talk page | Loading hangs | 1 | Design (D2–D4), caps + progress |
| 211 | Duplicate values | 1 | Design (D4) |
| 209, 207 | Run aborts on bad rows | 1 | Design (§3.4) |
| 204, 132, 2/91, 32 | Template parsing | 1 ✓ | Design (D5); #2 via the "unwrap nested templates" option (phase 2 ✓) |
| 178 | Unit leaks from permalink | 1 | Spec validation rejects unit not in allowed set |
| 175 | Better edit summary | 1 | D7 |
| 142, 140 | Browser-driven runs | 1 | Design (D1/D2) |
| 138, 137, 157, 141, 153 | Share page issues | 1 | POST + redirect, counts, spec JSON, sortable table |
| 104 | Local images imported | 1 | `imageinfo` repository check |
| 110 | Years with ≠4 digits | 1 | In new date parser |
| 97 | Self links | 1 | Guard in `item.rs` |
| 54 | Page title as value | 1 | Generalise existing `pagetitle` to all datatypes |
| 99 | Autoplay | 1 | Keep `run=` |
| 205 | Unnamed params docs | 1 | UI hint + docs |
| 40 | Multiple values per property | 1 | "skip if exact value exists" option |
| 210, 133 | Qualifiers | 2 ✓ | Fixed qualifier(s) per run; qualifier from another template param. Harvested value *as* a qualifier on a fixed main value (#133, first part) not done. |
| 16 | globe-coordinate | 2 ✓ | lat/lon params or `{{coord}}` style |
| 136, 15 | Unit suffix in quantity | 2 ✓ | Match unit label/alias/symbol |
| 135 | Calendar switch date | 2 | Full date instead of year |
| 130 | Archive URLs | 2 ✓ | Detect `web.archive.org`, option: skip / extract original |
| 70, 147, 52 | Case, separators, split/join | 2 ✓ | case option; value pattern `{1}-{2}`; punctuation alone counts as no value |
| 149 | Last link instead of first | 2 | Option |
| 172 | Formatter‑URL links in table | 2 ✓ | Use P1630 |
| 145 | Limit to instances of X | 2 | Batched VDQS filter on candidates |
| 71 | SPARQL / PetScan source | 2 | `PageSource::PetScan` (id) and `::Sparql` |
| 174 | Tags for shares | 2 | `share_tag` |
| 122 | Ignore templates far down | 2 ✓ | Option: lead section only |
| 56 | `wbparsevalue` | 2 | Use as validator/fallback for time & quantity |
| 118 | testwikis | 3 | Repo from `meta=wikibase`; parametrise Wikidata URLs |
| 206, 108, 111 | Multi‑wiki / multi‑template / multi‑property | 3 | Spec becomes a list; pipeline already per (page, property) |
| 89 | History check | 3 | Optional: scan last N summaries for a removed identical value |
| 53 | Commons linking | 3 | Via `wikibase_item` on Commons categories / P1472 |
| 146 | Voting on shares | 3 | Maybe; low value vs. complexity |
| 43 | i18n | 3 | ToolTranslate, UI strings only |
| 113 | Move images to Commons | — | Out of scope (decided) |
| 134 | Remove values on client wiki | — | Out of scope: destructive cross‑wiki edits |
| 203 | Values that aren't pages | — | User error; document "wikisyntax" option |
| 191, 212 | Empty / resolved | — | Close |

---

## 5. Phases

### Progress (2026-10-05)

- Phase 0: done.
- Phase 1 backend: done. Verified read-only against live enwiki/dewiki/Wikidata
  (replica and API page sources, WDQS, search, constraint checks); editing is
  tested against mocks only. Not deployed.
- Phase 1 frontend: done (form, run view with preview/start/stop, my runs,
  shared queries). Checked in headless Chromium against live wikis with
  `--dev-user`; real login and edits await deployment.

### Phase 0 — Skeleton (docs first)
- Cargo project, lint policy, `clippy.toml`, `rustfmt.toml`, CI (build, clippy `-D warnings`, `cargo test`, `cargo audit`).
- `config.rs` + `config.json.template`; `.gitignore` covers `config.json`, `oauth.ini`, `sessions/`.
- axum skeleton with health check, static `html/`, error type, tracing.
- `docs/ARCHITECTURE.md`, `docs/LOCAL_DEV.md`, `README.md` written **now**, kept current.

### Phase 1 — Parity + reliability (replaces the old tool)
- Auth (port from mixnmatch_rs), sessions, `require_user`.
- `wiki/`: site resolution, replica registry, `PageSource` (replica + API).
- `wikitext/` parser with table‑driven tests from real pages (including every parser issue above as a test case).
- `value/`: all current datatypes; month names/numerals as data files; date parser tests per language family.
- `constraints/`: all 23 checkers, local‑first.
- `wikidata/`: entity fetch, statement builder, edit with maxlag/back‑off, WDQS helpers.
- `harvest/`: spec, run, worker, pipeline, preview mode, CSV log.
- `storage/`: schema, runs, items, shares; share page.
- Frontend: form (same fields, grouped as today: Load / Define / Modify / Filter / Quality), results table with status colours, start/stop, permalink, save, download, progress.
- Deployment: Procfile, config on Toolforge, OAuth consumer, smoke test on a small category.

### Phase 2 — Most‑requested features
Qualifiers, coordinates, unit suffixes, transforms, formatter links, instance‑of filter, PetScan/SPARQL sources, tags, archive URLs, `wbparsevalue` validation.

### Phase 3 — Scale‑out
Multi‑wiki/template/property specs, testwikis, history check, i18n, optional separate job‑runner process.

---

## 6. Testing strategy

- **Parsers are pure**: `wikitext/`, `value/`, `transform.rs`, `summary.rs` — table‑driven unit tests, each GitHub issue becomes a named case.
- **HTTP**: `wiremock` fixtures for MW API, WDQS and Wikidata edits (`test_data/`). Pipeline tests run end‑to‑end against mocks, including maxlag retry (`tokio` `start_paused`).
- **Storage**: `testcontainers` MariaDB with `schema.sql`; one container per test process (pattern from `mixnmatch_rs::test_support`).
- **Replica SQL**: `#[ignore = "requires database / external services — run with cargo test -- --ignored"]` live tests through SSH tunnels.
- **Registries**: a test asserts every constraint QID and every API route is registered exactly once.
- CI runs the fast set; `cargo test -- --ignored` is a documented manual step.

---

## 7. Documentation (for maintainers)

| File | Content |
|---|---|
| `README.md` | What it is, screenshots, build/test/run in 10 lines, links below |
| `docs/ARCHITECTURE.md` | §3 of this plan, kept current; module map; run lifecycle diagram |
| `docs/HARVEST_PIPELINE.md` | Per‑candidate steps, every skip message and what triggers it |
| `docs/VALUE_PARSING.md` | Rules per datatype, date formats table, how to add a language |
| `docs/CONSTRAINTS.md` | Each constraint: QID, local or WDQS, known limitations |
| `docs/API.md` | JSON API reference with examples (also used by the frontend) |
| `docs/LOCAL_DEV.md` | Tunnels, config, `--dev-auth`, API page source, running tests |
| `docs/DEPLOY.md` | Toolforge build service, config path, OAuth consumer, restart, logs |
| `docs/DECISIONS.md` | D1–D10 above as short ADRs, appended as decisions change |
| `CONTRIBUTING.md` | Code style (CLAUDE.md rules), how to add a datatype/constraint/page source |

Code comments: doc comments on public items say *why*; no narrating comments.

---

## 8. Crates (initial)

`axum 0.8`, `tokio`, `tower-http` (trace, cors, timeout, compression),
`tower-sessions`, `reqwest` (gzip, cookies), `serde`/`serde_json`,
`mysql_async`, `clap`, `tracing`/`tracing-subscriber`, `thiserror`/`anyhow`,
`regex`, `fancy-regex`, `chrono`, `moka`, `dashmap`, `rand`, `uuid`, `csv`, `url`.
Dev: `wiremock`, `testcontainers` + `testcontainers-modules[mariadb]`,
`tempfile`, `tokio[test-util]`.
Evaluate before adding: `wikibase` (entity JSON types), `mediawiki` (API
continuation), `parse_wiki_text_2`.

---

## 9. Decisions taken

- Toolforge tool `harvesttemplates` exists; its database is
  `s58203__harvesttemplates` (created on first start).
- OAuth 2 client registered, callback `https://harvesttemplates.toolforge.org/callback`.
- Frontend: Vue 3, no build step.
- Repo and CI: GitHub (`magnusmanske/harvesttemplates`), GitHub Actions.
- #113 (move images to Commons): out of scope.
- OAuth tokens are **not** stored in the database. A running worker holds them
  in memory; after a restart, interrupted runs come back as *paused* and the
  owner resumes them from their (persistent) session.

Still open: an `ht_share` dump from the
`pltools` maintainers for migrating saved queries.
