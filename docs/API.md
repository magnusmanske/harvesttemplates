# HTTP API

JSON everywhere. Errors are `{"error": "message"}` with a matching status code
(400 invalid input, 401 not logged in, 403 not yours, 404, 429 too many active
runs, 500 with an error id that is also in the server log).
Mutating requests from other origins are refused.

## Auth

| Request | |
|---|---|
| `GET /api/auth/login?return_to=/path` | Redirects to MediaWiki OAuth 2, then back to `return_to` (local paths only). |
| `GET /callback` | OAuth callback; the path is taken from the registered callback URL. |
| `POST /api/auth/logout` | 204. |
| `GET /api/auth/me` | `{"user": "Name"}` or `{"user": null}`. |

## Form lookups

| Request | Returns |
|---|---|
| `GET /api/site?siteid=de&project=wikipedia` | dbname, language, namespaces, template/file/category prefixes, edition item |
| `GET /api/template?siteid=…&project=…&template=…` | `exists`, normalised `name`, `redirects`, `url` |
| `GET /api/property/P345` | label, datatype, `supported`, `deprecated`, `formatter_url` (P1630), `constraints` (one per type: id, label, status, supported), `units` (allowed units, `id: null` = no unit; `null` if unrestricted) |
| `GET /api/spec/from-query?<permalink params>` | a `JobSpec` from an old-style permalink |
| `POST /api/spec/to-query` with a `JobSpec` | `{"query": "siteid=…"}` |

## Runs

| Request | |
|---|---|
| `POST /api/runs` `{"spec": JobSpec, "share_id": null}` | Login required. Validates, then loads in the background. `{"id": 1}` |
| `GET /api/runs` | Your runs, newest first. |
| `GET /api/runs/{id}` | `run` (spec, status, editgroup, excluded, message, timestamps), `counts` by row status, `active`, `permalink` |
| `GET /api/runs/{id}/rows?status=error&offset=0&limit=100` | Rows (max 500 per page): seq, page_id, title, item, status, raw_value, value, message |
| `GET /api/runs/{id}/log.csv` | All rows as CSV. |
| `POST /api/runs/{id}/preview` | Owner only. Evaluate pending rows without editing. |
| `POST /api/runs/{id}/start` | Owner only. Edit pending and previewed rows as you. |
| `POST /api/runs/{id}/stop` | Owner only. Stops after the current row. |

Run status: `loading`, `ready`, `previewing`, `editing`, `paused`, `done`, `failed`.
Row status: `pending`, `ready` (previewed, would be added), `done`, `skipped`, `error`.

## Shares

| Request | |
|---|---|
| `GET /api/shares` | All shared queries with their tags and the outcome of their last complete run. The page `#/shares/TAG` lists one tag. |
| `GET /api/shares/{id}` | One share, including its spec. |
| `GET /api/shares/legacy/{htid}` | A share imported from the old tool, by its old id. |
| `POST /api/shares` `{"title": "…", "spec": JobSpec, "tags": ["…"]}` | Login required; the spec must validate. Tags are normalised (`Czech Wikipedia` → `czech-wikipedia`), at most 10. |
| `PUT /api/shares/{id}/tags` `{"tags": ["…"]}` | Creator only. Returns the normalised tags. |
| `DELETE /api/shares/{id}` | Creator only. |

## JobSpec

```json
{
  "siteid": "en", "project": "wikipedia", "namespace": 0,
  "property": "P345", "template": "IMDb title",
  "template_redirects": null,
  "parameters": ["id", "1"],
  "date_parameters": null,
  "use_page_title": false,
  "transform": {"add_prefix": "tt", "add_suffix": "", "remove_prefix": "", "remove_suffix": "", "search": "", "replace": ""},
  "plain_links": true, "link_choice": "first",
  "calendar": "gregorian",
  "date_limit": {"relation": "at_least", "date": {"year": 1926, "month": 0, "day": 0}},
  "unit": null, "decimal_mark": ".", "language": "",
  "category": "2024 films", "depth": 1, "manual_list": [],
  "skip_if": "property",
  "constraints": null,
  "qualifiers": [
    {"property": "P407", "source": "fixed", "value": "Q1860"},
    {"property": "P10135", "source": "parameter", "names": ["date"]}
  ]
}
```

All fields are optional; the defaults are shown. `template_redirects: null`
accepts every redirect; `constraints: null` checks every supported constraint.
`date_parameters` is `{"year": "…", "month": "…", "day": "…"}` (month and day optional).
Also: `skip_removed` (default `true`), `petscan` (a saved query's PSID: pages of the same wiki, or Wikidata items), `sparql` (a query selecting `?item`), `instance_of` (Q-ids; subclasses count), `archive_urls` (`original`, `skip`, `keep`), `value_pattern` (`"{1}-{2}"`), `unwrap_templates`, `lead_only` (booleans),
and `transform.case` (`unchanged`, `lower`, `upper`).
`coordinate_parameters` is `{"latitude": "…", "longitude": "…"}` (permalink: `latparam`, `lonparam`).
Fixed qualifier values are written as on Wikidata: `Q1860`, `2024-05-01`, `42`,
or `text@language` for monolingual text. In permalinks a qualifier is
`qualifier=P407|fixed|Q1860` or `qualifier=P10135|param|date,datum`.
