# HTTP API

JSON everywhere. Errors are `{"error": "message"}` with a matching status code
(400 invalid input, 401 not logged in, 403 not yours, 404, 429 too many active
runs, 500 with an error id that is also in the server log).
Mutating requests from other origins are refused.

## Auth

| Request | |
|---|---|
| `GET /api/auth/login?return_to=/path` | Redirects to MediaWiki OAuth, then back to `return_to` (local paths only). |
| `GET /api/auth/callback` | OAuth callback. |
| `POST /api/auth/logout` | 204. |
| `GET /api/auth/me` | `{"user": "Name"}` or `{"user": null}`. |

## Form lookups

| Request | Returns |
|---|---|
| `GET /api/site?siteid=de&project=wikipedia` | dbname, language, namespaces, template/file/category prefixes, edition item |
| `GET /api/template?siteid=…&project=…&template=…` | `exists`, normalised `name`, `redirects`, `url` |
| `GET /api/property/P345` | label, datatype, `supported`, `deprecated`, `constraints` (one per type: id, label, status, supported), `units` (allowed units, `id: null` = no unit; `null` if unrestricted) |
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
| `GET /api/shares` | All shared queries with the outcome of their last complete run. |
| `GET /api/shares/{id}` | One share, including its spec. |
| `POST /api/shares` `{"title": "…", "spec": JobSpec}` | Login required; the spec must validate. |
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
  "constraints": null
}
```

All fields are optional; the defaults are shown. `template_redirects: null`
accepts every redirect; `constraints: null` checks every supported constraint.
`date_parameters` is `{"year": "…", "month": "…", "day": "…"}` (month and day optional).
