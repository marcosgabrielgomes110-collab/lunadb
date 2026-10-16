# SDK Python (`lunapy`)

> Veja também: [API HTTP](api.md) · [Shell/CLI](shell.md) · [Operação](operations-pt.md)

Cliente oficial sem dependências externas (apenas stdlib). Fala com a API via HTTP.

## Instalação

```bash
pip install -e lunapy          # no repo
pip install -e "lunapy[dev]"   # + pytest
```

Requer Python ≥ 3.11.

## Criando o cliente

```python
from lunapy import LunaClient

# a partir do config.toml (~/.lunadb ou LUNADB_CONFIG)
client = LunaClient.from_config()

# ou explicitamente (env LUNADB_URL / LUNADB_API_KEY também são aceitos)
client = LunaClient(base_url="http://127.0.0.1:7700", api_key="luna_...")

# login (rotaciona a chave "login" e guarda no cliente)
user = client.login("root", "changeme")
print(user.username, user.role)
```

Todos os métodos levantam exceções tipadas (seção Erros).

## Métodos

### Sessão e sistema

| Método | Retorno |
|---|---|
| `health()` | `dict` |
| `login(username, password)` | `User` (grava `client.api_key`) |
| `rotate_key()` | `str` (nova key) |
| `me()` | `User` |
| `status()` | `dict` (admin) |

### API keys

```python
key, plaintext = client.create_api_key(name="ci")           # própria
key, plaintext = client.create_api_key(user.id, name="ci")  # admin, de outro usuário
client.list_api_keys()          # próprias -> list[ApiKey]
client.list_api_keys(user.id)   # admin
client.revoke_api_key(key.id)               # própria
client.revoke_api_key(key.id, user.id)      # admin (id ou prefixo)
```

### Usuários e roles (admin, exceto login/me)

```python
client.list_users()                                    # -> list[User]
client.create_user("bob", "bob12345", role="user", grants=[("app", "write")])  # -> User
client.set_grants(user.id, [Grant(db="app", level=Level.READ)])  # -> User
client.set_password(user.id, "novaSenha123", current_password="antiga123")  # -> User
client.delete_user(user.id)
client.list_roles()                                    # -> list[Role]
```

`grants` aceita `Grant`, dict `{"db":..., "level":...}`, tupla `("db","write")` ou lista `["db","write"]`.

### Databases

```python
client.list_dbs()        # -> list[DbMeta]
client.create_db("app")  # -> DbMeta (admin)
client.get_db("app")     # -> DbMeta
client.delete_db("app")
```

### Tabelas e schema

```python
from lunapy import TableSchema, Field, FieldType, AutoField

schema = TableSchema(
    strict=True,
    unique=("cpf", ("tenant", "email")),
    fields={
        "tenant": Field(FieldType.STRING, required=True),
        "email":  Field(FieldType.STRING, required=True, unique=True, pattern=r".+@.+\..+"),
        "senha":  Field(FieldType.STRING, required=True, encrypted=True, min_length=8),
        "idade":  Field(FieldType.INT, min=0, max=150),
        "ativo":  Field(FieldType.BOOL, default=True),
        "tags":   Field(FieldType.ARRAY, items=Field(FieldType.STRING)),
        "criado": Field(FieldType.DATETIME, auto=AutoField.CREATED_AT),
    },
)

client.create_table("app", "users", schema=schema)   # schema opcional
client.list_tables("app")                            # -> list[str]
client.get_schema("app", "users")                    # -> TableSchema
client.set_schema("app", "users", schema, force=False)  # -> TableSchema
client.delete_schema("app", "users")
client.delete_table("app", "users")
```

#### Migração incremental

Funciona também quando a tabela ainda não tem schema (cria do zero):

```python
# patch genérico (mesmo corpo da API PATCH)
client.patch_schema("app", "users", {"add": {"tel": {"type": "string"}}, "strict": False})

# atalhos
client.add_schema_fields("app", "users", {"tel": Field(FieldType.STRING, default="")})
client.update_schema_field("app", "users", "email", {"required": True, "max": None})
client.remove_schema_fields("app", "users", "tel", "campo_antigo")
client.add_unique_key("app", "users", "cpf")            # ou ("tenant", "email")
client.remove_unique_key("app", "users", "cpf")
```

- `add_schema_fields` aceita `Field` ou dict; `update_schema_field` faz merge (valor `None` limpa a propriedade).
- `force=True` aplica mesmo com registros inválidos; sem ele, violações levantam `LunaSchemaRejectedError` (`.records`).
- Operações inválidas (add existente, remove/update inexistente) → `LunaBadRequestError`.

`TableSchema.to_dict()/from_dict()` serializam para o formato da API; dicts crus também são aceitos
em `create_table`/`set_schema`.

### Registros

```python
record = client.create_record("app", "users", {"email": "a@x.com", "senha": "segredo1"})
client.get_record("app", "users", record["_id"])
client.replace_record("app", "users", record["_id"], {"email": "b@x.com", "senha": "outra123"})
client.patch_record("app", "users", record["_id"], {"idade": 31, "tel": None})  # -> dict (merge parcial)
client.list_records("app", "users", limit=10, offset=0)   # -> list[dict]
client.delete_record("app", "users", record["_id"])
client.validate_record("app", "users", {"email": "c@x.com"})  # -> dict (dry-run)
```

### Paginação por cursor

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
    filter={"idade": {"$gte": 18}},
    sort=["-idade"],
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
    filter={"$or": [{"idade": {"$gte": 18}}, {"tags": {"$contains": "vip"}}],
            "endereco.cidade": "SP"},
    sort=[Sort("idade", "desc"), "-nome"],   # aceita Sort, dict, tupla ("nome","asc") ou "-campo"
    fields=["nome", "endereco.cidade"],
    limit=20,
    offset=0,
)
result.total    # total filtrado
result.count    # retornados nesta página
result.records  # tuple[dict, ...]
```

### Agregações

```python
groups = client.aggregate(
    "app",
    "users",
    group_by=["cidade"],
    aggregates={
        "total": "count",                   # string "count"
        "receita": "sum:valor",             # string "op:campo"
        "ticket": {"op": "avg", "field": "valor"},  # ou dict
    },
    filter={"ativo": True},
)
# -> list[dict]: [{"key": {"cidade": "SP"}, "count": 2, "total": 2, "receita": 30, "ticket": 15.0}, ...]
```

Operadores e semântica: ver `docs/api.md` (seções Query e Agregações).

## Modelos (`lunapy.models`)

| Modelo | Campos |
|---|---|
| `Level` (str Enum) | `READ`, `WRITE`, `ADMIN` |
| `Grant` | `db`, `level` |
| `FieldType` (str Enum) | `STRING`, `INT`, `FLOAT`, `BOOL`, `DATETIME`, `DATE`, `UUID`, `OBJECT`, `ARRAY`, `ANY` |
| `AutoField` (str Enum) | `CREATED_AT`, `UPDATED_AT` |
| `Field` | `type`, `required`, `nullable`, `default`, `encrypted`, `unique`, `min`, `max`, `min_length`, `max_length`, `pattern`, `enum`, `items`, `fields`, `auto` |
| `TableSchema` | `fields`, `strict`, `unique` |
| `Sort` | `field`, `dir` (`"asc"`/`"desc"`) |
| `QueryResult` | `total`, `count`, `records` |
| `User` | `id`, `username`, `role`, `grants`, `created_at` |
| `Role` | `name`, `description`, `builtin`, `permissions` |
| `DbMeta` | `name`, `owner_id`, `created_at` |
| `ApiKey` | `id`, `name`, `prefix`, `created_at`, `last_used_at`, `revoked` |

Todos são dataclasses imutáveis (`frozen=True`) com `from_dict` (e `to_dict` em `Grant`, `Field`, `TableSchema`, `Sort`).

## Erros (`lunapy.errors`)

```
LunaError
├── LunaConnectionError          # falha de rede
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
    client.create_record("app", "users", {"email": "sem-arroba"})
except LunaValidationError as err:
    for problem in err.fields:
        print(problem["field"], problem["message"])
```

## Testes

```bash
cargo build                       # os testes pytest usam target/debug/lunadb
python -m pytest lunapy/tests
# opcional: LUNADB_BIN=/caminho/para/lunadb python -m pytest lunapy/tests
```

Os testes sobem o servidor real em uma porta e storage temporários (fixture de sessão) e
cobrem auth, CRUD, schema/unique, criptografia em disco, migração, queries, permissões,
api keys e os comandos `lunadb apikey ...`.
