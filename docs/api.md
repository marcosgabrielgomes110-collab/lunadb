# HTTP API

Default base: `http://127.0.0.1:7700` (configurable in `config.toml` → `[server] host/port`).

> See also: [Shell/CLI](shell.md) · [Python SDK](sdk.md)

## Authentication

All routes (except `GET /health` and `POST /auth/login`) require:

```
Authorization: Bearer luna_<64 hex>
```

Keys belong to a user and can be created/revoked individually
(name, prefix, `last_used_at`, revocation). The plaintext is only returned on creation/login.

## Errors

JSON body on all errors:

| Status | Meaning | Body |
|---|---|---|
| 400 | invalid request / validation | `{"error":"...","fields":[{"field":"email","message":"..."}]}` |
| 401 | missing/invalid API key | `{"error":"api key missing or invalid"}` |
| 403 | no permission on the DB or admin action | `{"error":"..."}` |
| 404 | not found | `{"error":"..."}` |
| 409 | conflict (unique, already exists, schema rejected) | `{"error":"...","fields":[...]}` or `{"error":"...","records":[{"id":"...","fields":[...]}]}` |
| 422 | malformed JSON / unknown field in body | extractor text |
| 429 | too many login attempts (temporary lockout) | `{"error":"..."}` + `Retry-After` header |
| 500 | internal error / encryption failure | `{"error":"..."}` |

Each request body is limited to `[server] max_body_bytes` (default 10 MiB) and
processing to `request_timeout_secs` (default 30s → `408`).

## System

### `GET /health` — public
```json
{"status":"ok","name":"lunadb","version":"0.0.1"}
```

### `GET /status` — admin
`pid`, `version`, `started_at`, `uptime_seconds`, `host`, `port`, `storage`,
`config`, `log_file`, `users`, `dbs`.

### `GET /metrics` — public
Counters in Prometheus text format (`text/plain`): `lunadb_build_info`,
`lunadb_uptime_seconds`, `lunadb_requests_total`, and `lunadb_responses_total{status_class="2xx|3xx|4xx|5xx"}`.
Aggregates only, no sensitive data.

## Auth and API keys

### `POST /auth/login` — public
```json
{"username":"root","password":"changeme"}
```
→ `200` `{"user":{...},"api_key":"luna_..."}`

Rotates **only** the user's key named `login`; the others remain valid.
After 5 consecutive failures for the same user, login is blocked for 60s
(returns `429` with `Retry-After`); a successful login resets the counter.

### `POST /auth/rotate-key` — auth
→ `{"api_key":"luna_..."}` (same semantics as login)

### `GET /auth/api-keys` — auth
Lists the current user's own keys:
```json
[{"id":"...","name":"login","prefix":"luna_abc123...","created_at":"...","last_used_at":null,"revoked":false}]
```

### `POST /auth/api-keys` — auth
```json
{"name":"ci"}
```
→ `201` `{"api_key":{...},"key":"luna_..."}` (plaintext shown only once; default `"default"`, max 50 active)

### `DELETE /auth/api-keys/{key_id|prefix}` — auth
→ `200` `{"id":"...","name":"ci","revoked":true,...}` (409 if already revoked or the prefix is ambiguous)

### `GET /me` — auth
→ `{"id","username","role","grants","created_at"}`

## Users (admin)

### `GET /users`
→ list of `UserPublic`.

### `POST /users`
```json
{"username":"bob","password":"bob12345","role":"user","grants":[{"db":"app","level":"write"}]}
```
→ `200` `UserPublic`. `role` defaults to `"user"`; minimum password length of 6 characters.

### `DELETE /users/{id}`
→ `204` (you cannot remove yourself or the last admin)

### `PUT /users/{id}/grants`
```json
{"grants":[{"db":"app","level":"read"},{"db":"*","level":"read"}]}
```
→ `UserPublic`

### `PUT /users/{id}/password`
```json
{"password":"newPassword123","current_password":"old123"}
```
→ `UserPublic`. An admin can change any user's password (without `current_password`);
a non-admin user must provide `current_password`. Minimum of 6 characters.
The change **revokes all active API keys** for the user. Changing the root password also
updates (and re-encrypts) `root.password` in `config.toml`.

### `GET|POST /users/{id}/api-keys` and `DELETE /users/{id}/api-keys/{key}` — admin
Same contract as the `/auth/api-keys` routes, but for any user.

### `GET /roles`
→ `[{"name","description","builtin","permissions":[...]}]`

**Grant levels:** `read` < `write` < `admin` (a DB admin manages tables/schema and can remove the DB).
The global `admin` role has full access (`*`) and exclusivity over users/roles/status.

## Databases

| Method | Route | Permission | Return |
|---|---|---|---|
| GET | `/dbs` | auth (filters by ACL) | `[DbMeta]` |
| POST | `/dbs` | **global admin** | `201` `DbMeta` |
| GET | `/dbs/{db}` | read | `DbMeta` |
| DELETE | `/dbs/{db}` | global admin or `admin` grant | `204` |

`DbMeta`:
```json
{"name":"app","owner_id":"...","created_at":"2026-10-10T03:02:52Z"}
```

## Tables and schema

| Method | Route | Permission | Return |
|---|---|---|---|
| GET | `/dbs/{db}/tables` | read | `{"db":"app","tables":["notes"]}` |
| POST | `/dbs/{db}/tables` | write | `201` `{"db":"app","table":"notes"}` |
| DELETE | `/dbs/{db}/tables/{table}` | write | `204` |
| GET | `/dbs/{db}/tables/{table}/schema` | read | `TableSchema` (404 if none) |
| PUT | `/dbs/{db}/tables/{table}/schema` | write | `TableSchema` (409 if records violate it) |
| PATCH | `/dbs/{db}/tables/{table}/schema` | write | `TableSchema` (incremental migration) |
| DELETE | `/dbs/{db}/tables/{table}/schema` | write | `204` (decrypts encrypted fields) |

`POST /dbs/{db}/tables` accepts an inline schema:
```json
{
  "name": "users",
  "schema": {
    "strict": true,
    "unique": ["cpf", ["tenant_id", "email"]],
    "fields": {"email": {"type": "string", "required": true}}
  }
}
```

`PUT .../schema?force=true` applies even with invalid records.

### Incremental migration (`PATCH .../schema`)

Applies operations to the current schema — including when the table **has no schema yet**
(creates it from scratch). The entire record is migrated/re-encrypted as in `PUT`.

```json
{
  "add": {"phone": {"type": "string", "required": false}},
  "update": {"email": {"required": true, "max_length": 120}},
  "remove": ["old_field"],
  "unique_add": [["tenant_id", "email"]],
  "unique_remove": ["cpf"],
  "strict": true
}
```

| Operation | Semantics |
|---|---|
| `add` | inserts new fields; 400 error if the field already exists (use `update`) |
| `update` | shallow merge into the field's properties; `null` clears the property (e.g. `{"max": null}`); 400 error if it does not exist |
| `remove` | removes fields; 400 error if it does not exist |
| `unique_add` / `unique_remove` | simple unique keys (`"email"`) or composite ones (`["a","b"]`) |
| `strict` | sets strict mode |

- Fields removed/unknown under `strict: false` remain in the records (no data is deleted).
- Invalid records after the operation → `409` with the ids (`?force=true` applies anyway).
- Unknown operation/key in the body → `422`; without `write` permission → `403`; table does not exist → `404`.

### Schema DSL

```json
{
  "strict": true,
  "unique": ["cpf", ["tenant_id", "email"]],
  "fields": {
    "email":      {"type":"string","required":true,"pattern":".+@.+\\..+","unique":true},
    "password":   {"type":"string","required":true,"encrypted":true,"min_length":8},
    "age":        {"type":"int","min":0,"max":150},
    "active":     {"type":"bool","default":true},
    "tags":       {"type":"array","items":{"type":"string"}},
    "address":    {"type":"object","fields":{"city":{"type":"string","required":true}}},
    "external_id":{"type":"uuid"},
    "birthdate":  {"type":"date"},
    "created":    {"type":"datetime","auto":"created_at"},
    "updated":    {"type":"datetime","auto":"updated_at"},
    "meta":       {"type":"any"}
  }
}
```

- **Types**: `string`, `int`, `float`, `bool`, `datetime` (RFC3339), `date` (`YYYY-MM-DD`), `uuid`, `object`, `array`, `any`.
- **Fields**: `required`, `nullable`, `default`, `unique` (top level only, never `encrypted`), `encrypted` (top level only), `min`/`max` (numeric), `min_length`/`max_length` (string/array), `pattern` (regex), `enum`, `items`, `fields`, `auto` (`created_at`/`updated_at`, datetime only).
- **Table `unique`**: each item is a field (`"cpf"`) or a combination (`["tenant_id","email"]`). A null component is ignored.
- **`encrypted`**: value encrypted on disk (AES-256-GCM) and transparently decrypted in the API; it remains filterable in queries, but cannot be `unique`.
- **`strict`**: rejects undeclared fields.
- Migration: `PUT schema` revalidates and re-encrypts records using the old schema; a violation → `409` with the ids.

## Records

| Method | Route | Permission | Return |
|---|---|---|---|
| GET | `/dbs/{db}/tables/{table}/records?limit=&offset=&cursor=` | read | `{"db","table","total","count","next_cursor","records":[{"_id":...}]}` |
| POST | `/dbs/{db}/tables/{table}/records` | write | `201` record with `_id` |
| POST | `/dbs/{db}/tables/{table}/records/validate` | write | `200` prepared record (dry-run, does not persist) |
| POST | `/dbs/{db}/tables/{table}/records/query` | read | `{"db","table","total","count","next_cursor","records"}` |
| POST | `/dbs/{db}/tables/{table}/records/aggregate` | read | `{"db","table","total","count","groups"}` |
| GET | `/dbs/{db}/tables/{table}/records/{id}` | read | record |
| PUT | `/dbs/{db}/tables/{table}/records/{id}` | write | record (full replacement) |
| PATCH | `/dbs/{db}/tables/{table}/records/{id}` | write | record (partial merge) |
| DELETE | `/dbs/{db}/tables/{table}/records/{id}` | write | `204` |

The record body must be a JSON object; `_id` is generated by the server and ignored if sent.
`total` is the table size and `count` the number returned on the page; `limit` is capped at 1000
(default), avoiding unbounded responses.

### Cursor pagination (`next_cursor` / `cursor`)

Both listing and query accept an opaque `cursor` and return `next_cursor`. The cursor is a
base64url-encoded **keyset** position (the sort values of the last record, plus `_id` as a
unique tiebreaker), not an integer offset. Consequences:

- A walk never skips or repeats a record, even if rows are inserted or removed between pages.
- `total` is the count before the cursor window, so it stays the same for every page of a walk.
- `_id` is always appended as the final sort key. Records with a missing or null value for a
  sort field still sort last, and the tiebreaker keeps the order total.
- When `sort` is omitted, records are returned ordered by `_id` ascending.
- `next_cursor` is present only while more pages remain; a short final page omits it.
- `offset` still works and is applied to the first page when no `cursor` is supplied. Sending
  both `cursor` and `offset` uses the cursor and ignores the offset.
- A malformed cursor, or one that does not match the current `sort`, returns `400`.

```bash
# first page
curl -s "$BASE/dbs/app/tables/notes/records?limit=50" -H "authorization: Bearer $KEY"
# -> {"total":1200,...,"next_cursor":"eyJrIjpbLi4uXX0"}

# next page
curl -s "$BASE/dbs/app/tables/notes/records?limit=50&cursor=eyJrIjpbLi4uXX0" \
  -H "authorization: Bearer $KEY"
```

The same applies to `POST .../records/query`, which takes `cursor` in the JSON body.

### Partial merge (`PATCH .../records/{id}`)

Applies a [merge patch](https://datatracker.ietf.org/doc/html/rfc7386): objects are merged
recursively, `null` removes the key, and other values replace. The resulting record goes
through the same validation/uniqueness as `PUT`, preserving `auto: created_at` fields.

```json
{"age": 31, "address": {"city": "RJ"}, "phone": null}
```

### Aggregations (`POST .../records/aggregate`)

```json
{
  "filter": {"active": true},
  "group_by": ["city"],
  "aggregates": {
    "total": {"op": "count"},
    "revenue": {"op": "sum", "field": "value"},
    "ticket": {"op": "avg", "field": "value"},
    "lowest": {"op": "min", "field": "value"},
    "highest": {"op": "max", "field": "value"}
  },
  "limit": 50
}
```

Returns `groups`, each with `key` (the `group_by` values), `count`, and the `aggregates`
names. Operations: `count`, `sum`, `avg`, `min`, `max` (the numeric ones require `field`).
`total` is the number of records that passed the filter. Groups are sorted by key.

### Query

```json
{
  "filter": {"$or": [{"age": {"$gte": 18}}, {"tags": {"$contains": "vip"}}],
             "address.city": "SP"},
  "sort": ["-age", {"field": "name", "dir": "asc"}],
  "fields": ["name", "address.city"],
  "limit": 20,
  "offset": 0,
  "cursor": "eyJrIjpbLi4uXX0"
}
```

- **Operators**: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$exists`, `$contains` (substring on a string / membership in an array), `$starts_with`, `$ends_with`, `$regex`, `$and`, `$or`, `$not`. Equality is implicit: `{"tenant": "t1"}`.
- **Paths**: `a.b` (objects) and `tags.0` (array index). Filtering by `_id` works.
- **Sort**: `"field"`, `"-field"` (desc), or `{"field","dir"}`; null/missing values always last. Defaults to `_id` ascending.
- **`fields`**: projection (accepts nested paths); `_id` always included.
- **`total`**: filtered total before `limit`/`offset`/`cursor`.
- **`cursor`/`next_cursor`**: keyset pagination; see [Cursor pagination](#cursor-pagination-next_cursor--cursor).
- `encrypted` fields can be filtered (decrypted in memory).
- Unknown fields in the body → `422`; invalid operator/filter/cursor → `400`.

## Quick example

```bash
BASE=http://127.0.0.1:7700
KEY=$(curl -s -X POST $BASE/auth/login -H 'content-type: application/json' \
  -d '{"username":"root","password":"changeme"}' | jq -r .api_key)

curl -s -X POST $BASE/dbs -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"name":"app"}'

curl -s -X POST $BASE/dbs/app/tables -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"name":"notes"}'

curl -s -X POST $BASE/dbs/app/tables/notes/records -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"title":"first"}'

curl -s -X POST $BASE/dbs/app/tables/notes/records/query -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"filter":{"title":{"$contains":"first"}}}'
```
