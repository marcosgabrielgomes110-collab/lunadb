# Python SDK (`lunapy`)

> See also: [HTTP API](api.md) · [Shell/CLI](shell.md) · [Operations](operations.md)

Official client with no external dependencies (stdlib only). Talks to the API over HTTP.

## Installation

```bash
pip install -e lunapy          # in the repo
pip install -e "lunapy[dev]"   # + pytest
```

Requires Python ≥ 3.11.

## Creating the client

```python
from lunapy import LunaClient

# from config.toml (~/.lunadb or LUNADB_CONFIG)
client = LunaClient.from_config()

# or explicitly (env LUNADB_URL / LUNADB_API_KEY are also accepted)
client = LunaClient(base_url="http://127.0.0.1:7700", api_key="luna_...")

# login (rotates the "login" key and stores it on the client)
user = client.login("root", "changeme")
print(user.username, user.role)
```

`connect()` creates the client and logs in in one step; clients also work as
context managers. All methods raise typed exceptions (see the Errors section).

```python
from lunapy import connect

with connect(base_url="http://127.0.0.1:7700", username="root", password="changeme") as client:
    print(client.me().username)
```

## Handles (less repetition)

`client.db()` and `client.table()` return lightweight handles that remember the
database/table name. Every low-level method still exists and behaves the same.

```python
app = client.db("app")             # or client["app"]
users = app.create_table("users")  # -> Table handle
users = app["users"]               # handle to an existing table

users.create(name="Ana", age=30)   # kwargs become record fields
record = users.first(name="Ana")   # -> Record
print(record.name, record["age"])

users.update(record["_id"], age=31)        # partial update
users.replace(record["_id"], name="Ana", age=32)
users.delete(record["_id"])

for person in users:                        # keyset pagination under the hood
    print(person.name)

len(users)                # users.count()
record["_id"] in users    # existence check
users[record["_id"]]      # users.get(id)
```

`Record` is a `dict` subclass, so `record["name"]` keeps working; attribute access
is a shortcut (`record.name`). Dict methods win on name clashes, so use
`record["items"]` for a field literally called `items`.

### Chainable queries

```python
for user in users.where(age__gte=18).sort("-age").select("name", "age").limit(50):
    print(user.name)

users.where(tags__contains="vip").count()
users.where(city="SP").first()       # -> Record | None
users.where(age__gte=18).ids()       # -> list of _id
users.where(age__gte=18).one()       # raises LunaNotFoundError when empty
```

Keyword operators: `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `in`, `nin`, `contains`,
`startswith`, `ends_with`, `regex`, `exists`. Use an explicit `filter={...}` for
`$and`/`$or`/`$not` or dotted field names (`"address.city"`).

### Schema shortcuts

```python
schema = TableSchema.of(
    email=Field.string(required=True, unique=True, pattern=r".+@.+\..+"),
    age=Field.int(min=0),
    active=Field.bool(default=True),
    tags=Field.array(items="string"),
)
users.set_schema(schema, force=True)

users.add_fields({"nickname": "string"})
users.update_field("nickname", {"required": True}, force=True)
users.add_unique("nickname", force=True)
users.remove_unique("nickname", force=True)
users.remove_fields("nickname")
```

`Field` accepts either a `FieldType` or its string (`Field("bool")`), and
`TableSchema` accepts strings/raw dicts as field definitions. A single unique key
does not need nesting: `TableSchema(unique="email")`.

## Methods

### Session and system

| Method | Return |
|---|---|
| `health()` | `dict` |
| `login(username, password)` | `User` (writes `client.api_key`) |
| `connect(...)` (classmethod/module function) | `LunaClient` (optional login) |
| `rotate_key()` | `str` (new key) |
| `me()` | `User` |
| `status()` | `dict` (admin) |
| `db(name)` / `database(name)` | `Database` handle |
| `table(db, table)` | `Table` handle |

### API keys

```python
key, plaintext = client.create_api_key(name="ci")           # own
key, plaintext = client.create_api_key(user.id, name="ci")  # admin, another user's
client.list_api_keys()          # own -> list[ApiKey]
client.list_api_keys(user.id)   # admin
client.revoke_api_key(key.id)               # own
client.revoke_api_key(key.id, user.id)      # admin (id or prefix)
```

### Users and roles (admin, except login/me)

```python
client.list_users()                                    # -> list[User]
client.create_user("bob", "bob12345", role="user", grants=[("app", "write")])  # -> User
client.set_grants(user.id, [Grant(db="app", level=Level.READ)])  # -> User
client.set_password(user.id, "newPassword123", current_password="old123")  # -> User
client.delete_user(user.id)
client.list_roles()                                    # -> list[Role]
```

`grants` accepts a `Grant`, a dict `{"db":..., "level":...}`, a tuple `("db","write")`, or a list `["db","write"]`.

### Databases

```python
client.list_dbs()        # -> list[DbMeta]
client.create_db("app")  # -> DbMeta (admin)
client.get_db("app")     # -> DbMeta
client.delete_db("app")
```

### Tables and schema

```python
from lunapy import TableSchema, Field, FieldType, AutoField

schema = TableSchema(
    strict=True,
    unique=("cpf", ("tenant", "email")),
    fields={
        "tenant":   Field(FieldType.STRING, required=True),
        "email":    Field(FieldType.STRING, required=True, unique=True, pattern=r".+@.+\..+"),
        "password": Field(FieldType.STRING, required=True, encrypted=True, min_length=8),
        "age":      Field(FieldType.INT, min=0, max=150),
        "active":   Field(FieldType.BOOL, default=True),
        "tags":     Field(FieldType.ARRAY, items=Field(FieldType.STRING)),
        "created":  Field(FieldType.DATETIME, auto=AutoField.CREATED_AT),
    },
)

client.create_table("app", "users", schema=schema)   # schema optional
client.list_tables("app")                            # -> list[str]
client.get_schema("app", "users")                    # -> TableSchema
client.set_schema("app", "users", schema, force=False)  # -> TableSchema
client.delete_schema("app", "users")
client.delete_table("app", "users")
```

#### Incremental migration

Works also when the table has no schema yet (creates it from scratch):

```python
# generic patch (same body as the API PATCH)
client.patch_schema("app", "users", {"add": {"tel": {"type": "string"}}, "strict": False})

# shortcuts
client.add_schema_fields("app", "users", {"tel": Field(FieldType.STRING, default="")})
client.update_schema_field("app", "users", "email", {"required": True, "max": None})
client.remove_schema_fields("app", "users", "tel", "old_field")
client.add_unique_key("app", "users", "cpf")            # or ("tenant", "email")
client.remove_unique_key("app", "users", "cpf")
```

- `add_schema_fields` accepts a `Field` or a dict; `update_schema_field` merges (a `None` value clears the property).
- `force=True` applies even with invalid records; without it, violations raise `LunaSchemaRejectedError` (`.records`).
- Invalid operations (adding an existing one, removing/updating a nonexistent one) → `LunaBadRequestError`.

`TableSchema.to_dict()/from_dict()` serialize to the API format; raw dicts are also accepted
in `create_table`/`set_schema`.

### Records

```python
record = client.create_record("app", "users", {"email": "a@x.com", "password": "secret1"})
record = client.create_record("app", "users", email="a@x.com", password="secret1")  # kwargs
client.get_record("app", "users", record["_id"])
client.replace_record("app", "users", record["_id"], {"email": "b@x.com", "password": "another123"})
client.patch_record("app", "users", record["_id"], {"age": 31, "tel": None})  # -> Record (partial merge)
client.update_record("app", "users", record["_id"], age=32)                    # alias + kwargs
client.list_records("app", "users", limit=10, offset=0)   # -> list[Record]
client.count("app", "users")                              # -> int
client.exists("app", "users", record["_id"])              # -> bool
client.delete_record("app", "users", record["_id"])
client.validate_record("app", "users", {"email": "c@x.com"})  # -> Record (dry-run)
```

### Cursor pagination

`list_records_page` devolve a página inteira, com o cursor para a próxima:

```python
page = client.list_records_page("app", "users", limit=100)
page.total        # tamanho da tabela, igual em todas as páginas
page.count        # registros nesta página
page.records      # tuple[dict, ...]
page.next_cursor  # str | None (None na última página)

# próxima página
page = client.list_records_page("app", "users", limit=100, cursor=page.next_cursor)
```

Os iteradores cuidam do cursor para você e são estáveis sob inserções/remoções concorrentes
(diferente do `offset`):

```python
for record in client.iter_records("app", "users", page_size=200):
    ...

# com filtro e ordenação
for record in client.iter_query(
    "app", "users",
    filter={"age": {"$gte": 18}},
    sort=["-age"],
    page_size=200,
    limit=5000,        # opcional: para depois de N registros
):
    ...
```

`list_records` continua retornando só a lista de registros, para compatibilidade.

### Query

```python
from lunapy import Sort

result = client.query(
    "app",
    "users",
    filter={"$or": [{"age": {"$gte": 18}}, {"tags": {"$contains": "vip"}}],
            "address.city": "SP"},
    sort=[Sort("age", "desc"), "-name"],   # accepts Sort, dict, ("name","asc") tuple, or "-field"
    fields=["name", "address.city"],
    limit=20,
    offset=0,
)
result.total    # filtered total
result.count    # returned on this page
result.records  # tuple[dict, ...]
```

### Aggregations

```python
groups = client.aggregate(
    "app",
    "users",
    group_by=["city"],
    aggregates={
        "total": "count",                   # "count" string
        "revenue": "sum:value",             # "op:field" string
        "ticket": {"op": "avg", "field": "value"},  # or dict
    },
    filter={"active": True},
)
# -> list[dict]: [{"key": {"city": "SP"}, "count": 2, "total": 2, "revenue": 30, "ticket": 15.0}, ...]
```

Operators and semantics: see `docs/api.md` (Query and Aggregations sections).

## Models (`lunapy.models`)

| Model | Fields |
|---|---|
| `Level` (str Enum) | `READ`, `WRITE`, `ADMIN` |
| `Grant` | `db`, `level` |
| `FieldType` (str Enum) | `STRING`, `INT`, `FLOAT`, `BOOL`, `DATETIME`, `DATE`, `UUID`, `OBJECT`, `ARRAY`, `ANY` |
| `AutoField` (str Enum) | `CREATED_AT`, `UPDATED_AT` |
| `Field` | `type`, `required`, `nullable`, `default`, `encrypted`, `unique`, `min`, `max`, `min_length`, `max_length`, `pattern`, `enum`, `items`, `fields`, `auto` |
| `TableSchema` | `fields`, `strict`, `unique` |
| `Sort` | `field`, `dir` (`"asc"`/`"desc"`); `Sort.asc()`/`Sort.desc()` |
| `Record` (`dict` subclass) | dynamic; `record.field`, `record["field"]`, `to_dict()` |
| `Page` | `total`, `count`, `records`, `next_cursor`, `first` |
| `QueryResult` / `RecordPage` | `Page` subclasses used by `query()` / `list_records_page()` |
| `User` | `id`, `username`, `role`, `grants`, `created_at` |
| `Role` | `name`, `description`, `builtin`, `permissions` |
| `DbMeta` | `name`, `owner_id`, `created_at` |
| `ApiKey` | `id`, `name`, `prefix`, `created_at`, `last_used_at`, `revoked` |

All are immutable dataclasses (`frozen=True`) with `from_dict` (and `to_dict` on
`Grant`, `Field`, `TableSchema`, `Sort`). `Field` also offers typed shortcuts:
`Field.string(...)`, `Field.int(...)`, `Field.bool(...)`, `Field.array(...)`,
`Field.datetime(...)`, among others.

## Errors (`lunapy.errors`)

```
LunaError
├── LunaConnectionError          # network failure
└── LunaHTTPError                # .status, .message, .payload
    ├── LunaBadRequestError      # 400
    │   └── LunaValidationError  # .fields [{field, message}]
    ├── LunaAuthError            # 401
    ├── LunaForbiddenError       # 403
    ├── LunaNotFoundError        # 404
    └── LunaConflictError        # 409
        ├── LunaUniqueViolationError   # .fields
        └── LunaSchemaRejectedError    # .records [{id, fields}]
```

```python
from lunapy import LunaUniqueViolationError, LunaValidationError

try:
    client.create_record("app", "users", {"email": "no-at-sign"})
except LunaValidationError as err:
    for problem in err.fields:
        print(problem["field"], problem["message"])
```

## Tests

```bash
cargo build                       # the pytest tests use target/debug/lunadb
python -m pytest lunapy/tests
# optional: LUNADB_BIN=/path/to/lunadb python -m pytest lunapy/tests
```

The tests start the real server on a temporary port and storage (session fixture) and
cover auth, CRUD, schema/unique, encryption at rest, migration, queries, permissions,
API keys, the `lunadb apikey ...` commands, and the Pythonic shortcuts (handles,
`Record`, query builder, and abbreviated schemas).
