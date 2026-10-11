# API HTTP

Base padrão: `http://127.0.0.1:7700` (configurável em `config.toml` → `[server] host/port`).

> Veja também: [Shell/CLI](shell.md) · [SDK Python](sdk.md)

## Autenticação

Todas as rotas (exceto `GET /health` e `POST /auth/login`) exigem:

```
Authorization: Bearer luna_<64 hex>
```

As keys pertencem a um usuário e podem ser criadas/revogadas individualmente
(nome, prefixo, `last_used_at`, revogação). O plaintext só é retornado na criação/login.

## Erros

Corpo JSON em todos os erros:

| Status | Significado | Corpo |
|---|---|---|
| 400 | requisição inválida / validação | `{"error":"...","fields":[{"field":"email","message":"..."}]}` |
| 401 | API key ausente/inválida | `{"error":"api key ausente ou invalida"}` |
| 403 | sem permissão no DB ou ação admin | `{"error":"..."}` |
| 404 | não encontrado | `{"error":"..."}` |
| 409 | conflito (unique, já existe, schema rejeitado) | `{"error":"...","fields":[...]}` ou `{"error":"...","records":[{"id":"...","fields":[...]}]}` |
| 422 | JSON malformado/campo desconhecido no corpo | texto do extractor |
| 500 | erro interno / falha de criptografia | `{"error":"..."}` |

## Sistema

### `GET /health` — público
```json
{"status":"ok","name":"lunadb","version":"0.0.1"}
```

### `GET /status` — admin
`pid`, `version`, `started_at`, `uptime_seconds`, `host`, `port`, `storage`,
`config`, `log_file`, `users`, `dbs`.

## Auth e API keys

### `POST /auth/login` — público
```json
{"username":"root","password":"changeme"}
```
→ `200` `{"user":{...},"api_key":"luna_..."}`

Rotaciona **apenas** a chave de nome `login` do usuário; as demais continuam válidas.

### `POST /auth/rotate-key` — auth
→ `{"api_key":"luna_..."}` (mesma semântica do login)

### `GET /auth/api-keys` — auth
Lista as chaves do próprio usuário:
```json
[{"id":"...","name":"login","prefix":"luna_abc123...","created_at":"...","last_used_at":null,"revoked":false}]
```

### `POST /auth/api-keys` — auth
```json
{"name":"ci"}
```
→ `201` `{"api_key":{...},"key":"luna_..."}` (plaintext exibido uma única vez; default `"default"`, máx. 50 ativas)

### `DELETE /auth/api-keys/{key_id|prefixo}` — auth
→ `200` `{"id":"...","name":"ci","revoked":true,...}` (409 se já revogada ou prefixo ambíguo)

### `GET /me` — auth
→ `{"id","username","role","grants","created_at"}`

## Usuários (admin)

### `GET /users`
→ lista de `UserPublic`.

### `POST /users`
```json
{"username":"bob","password":"bob12345","role":"user","grants":[{"db":"app","level":"write"}]}
```
→ `200` `UserPublic`. `role` default `"user"`; senha mínima de 6 caracteres.

### `DELETE /users/{id}`
→ `204` (não pode remover a si mesmo nem o último admin)

### `PUT /users/{id}/grants`
```json
{"grants":[{"db":"app","level":"read"},{"db":"*","level":"read"}]}
```
→ `UserPublic`

### `GET|POST /users/{id}/api-keys` e `DELETE /users/{id}/api-keys/{key}` — admin
Mesmo contrato das rotas `/auth/api-keys`, porém para qualquer usuário.

### `GET /roles`
→ `[{"name","description","builtin","permissions":[...]}]`

**Níveis de grant:** `read` < `write` < `admin` (admin do DB gerencia tabelas/schema e pode remover o DB).
Role `admin` global tem acesso total (`*`) e exclusividade em usuários/roles/status.

## Databases

| Método | Rota | Permissão | Retorno |
|---|---|---|---|
| GET | `/dbs` | auth (filtra por ACL) | `[DbMeta]` |
| POST | `/dbs` | **admin global** | `201` `DbMeta` |
| GET | `/dbs/{db}` | read | `DbMeta` |
| DELETE | `/dbs/{db}` | admin global ou grant `admin` | `204` |

`DbMeta`:
```json
{"name":"app","owner_id":"...","created_at":"2026-10-10T03:02:52Z"}
```

## Tabelas e schema

| Método | Rota | Permissão | Retorno |
|---|---|---|---|
| GET | `/dbs/{db}/tables` | read | `{"db":"app","tables":["notas"]}` |
| POST | `/dbs/{db}/tables` | write | `201` `{"db":"app","table":"notas"}` |
| DELETE | `/dbs/{db}/tables/{table}` | write | `204` |
| GET | `/dbs/{db}/tables/{table}/schema` | read | `TableSchema` (404 se não houver) |
| PUT | `/dbs/{db}/tables/{table}/schema` | write | `TableSchema` (409 se registros violarem) |
| PATCH | `/dbs/{db}/tables/{table}/schema` | write | `TableSchema` (migração incremental) |
| DELETE | `/dbs/{db}/tables/{table}/schema` | write | `204` (decifra campos encrypted) |

`POST /dbs/{db}/tables` aceita schema inline:
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

`PUT .../schema?force=true` aplica mesmo com registros inválidos.

### Migração incremental (`PATCH .../schema`)

Aplica operações sobre o schema atual — inclusive quando a tabela **ainda não tem schema**
(cria do zero). O registro inteiro é migrado/recifrado como no `PUT`.

```json
{
  "add": {"telefone": {"type": "string", "required": false}},
  "update": {"email": {"required": true, "max_length": 120}},
  "remove": ["campo_antigo"],
  "unique_add": [["tenant_id", "email"]],
  "unique_remove": ["cpf"],
  "strict": true
}
```

| Operação | Semântica |
|---|---|
| `add` | insere campos novos; erro 400 se o campo já existir (use `update`) |
| `update` | merge raso nas propriedades do campo; `null` limpa a propriedade (ex.: `{"max": null}`); erro 400 se não existir |
| `remove` | remove campos; erro 400 se não existir |
| `unique_add` / `unique_remove` | chaves únicas simples (`"email"`) ou compostas (`["a","b"]`) |
| `strict` | define o modo estrito |

- Campos removidos/desconhecidos em `strict: false` permanecem nos registros (nenhum dado é apagado).
- Registros inválidos após a operação → `409` com os ids (`?force=true` aplica mesmo assim).
- Operação/chave desconhecida no corpo → `422`; sem permissão `write` → `403`; tabela inexistente → `404`.

### DSL de schema

```json
{
  "strict": true,
  "unique": ["cpf", ["tenant_id", "email"]],
  "fields": {
    "email":      {"type":"string","required":true,"pattern":".+@.+\\..+","unique":true},
    "senha":      {"type":"string","required":true,"encrypted":true,"min_length":8},
    "idade":      {"type":"int","min":0,"max":150},
    "ativo":      {"type":"bool","default":true},
    "tags":       {"type":"array","items":{"type":"string"}},
    "endereco":   {"type":"object","fields":{"cidade":{"type":"string","required":true}}},
    "external_id":{"type":"uuid"},
    "nascimento": {"type":"date"},
    "criado":     {"type":"datetime","auto":"created_at"},
    "atualizado": {"type":"datetime","auto":"updated_at"},
    "meta":       {"type":"any"}
  }
}
```

- **Tipos**: `string`, `int`, `float`, `bool`, `datetime` (RFC3339), `date` (`YYYY-MM-DD`), `uuid`, `object`, `array`, `any`.
- **Campos**: `required`, `nullable`, `default`, `unique` (só topo, nunca `encrypted`), `encrypted` (só topo), `min`/`max` (numéricos), `min_length`/`max_length` (string/array), `pattern` (regex), `enum`, `items`, `fields`, `auto` (`created_at`/`updated_at`, só datetime).
- **`unique` de tabela**: cada item é um campo (`"cpf"`) ou combinação (`["tenant_id","email"]`). Componente nulo é ignorado.
- **`encrypted`**: valor cifrado em disco (AES-256-GCM) e decifrado transparente na API; continua filtrável em queries, mas não pode ser `unique`.
- **`strict`**: rejeita campos não declarados.
- Migração: `PUT schema` revalida e recifra os registros pela schema antiga; violação → `409` com os ids.

## Registros

| Método | Rota | Permissão | Retorno |
|---|---|---|---|
| GET | `/dbs/{db}/tables/{table}/records?limit=&offset=` | read | `{"db","table","count","records":[{"_id":...}]}` |
| POST | `/dbs/{db}/tables/{table}/records` | write | `201` registro com `_id` |
| POST | `/dbs/{db}/tables/{table}/records/validate` | write | `200` registro preparado (dry-run, não persiste) |
| POST | `/dbs/{db}/tables/{table}/records/query` | read | `{"db","table","total","count","records"}` |
| GET | `/dbs/{db}/tables/{table}/records/{id}` | read | registro |
| PUT | `/dbs/{db}/tables/{table}/records/{id}` | write | registro (substituição total) |
| DELETE | `/dbs/{db}/tables/{table}/records/{id}` | write | `204` |

O corpo do registro deve ser um objeto JSON; `_id` é gerado pelo servidor e ignorado se enviado.

### Query

```json
{
  "filter": {"$or": [{"idade": {"$gte": 18}}, {"tags": {"$contains": "vip"}}],
             "endereco.cidade": "SP"},
  "sort": ["-idade", {"field": "nome", "dir": "asc"}],
  "fields": ["nome", "endereco.cidade"],
  "limit": 20,
  "offset": 0
}
```

- **Operadores**: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$exists`, `$contains` (substring em string / pertence em array), `$starts_with`, `$ends_with`, `$regex`, `$and`, `$or`, `$not`. Igualdade é implícita: `{"tenant": "t1"}`.
- **Caminhos**: `a.b` (objetos) e `tags.0` (índice de array). Filtro por `_id` funciona.
- **Sort**: `"campo"`, `"-campo"` (desc) ou `{"field","dir"}`; nulos/ausentes sempre por último.
- **`fields`**: projeção (aceita caminhos aninhados); `_id` sempre incluído.
- **`total`**: total filtrado antes de `limit`/`offset`.
- Campos `encrypted` podem ser filtrados (decifra em memória).
- Campos desconhecidos no corpo → `422`; operador/filtro inválido → `400`.

## Exemplo rápido

```bash
BASE=http://127.0.0.1:7700
KEY=$(curl -s -X POST $BASE/auth/login -H 'content-type: application/json' \
  -d '{"username":"root","password":"changeme"}' | jq -r .api_key)

curl -s -X POST $BASE/dbs -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"name":"app"}'

curl -s -X POST $BASE/dbs/app/tables -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"name":"notas"}'

curl -s -X POST $BASE/dbs/app/tables/notas/records -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"titulo":"primeira"}'

curl -s -X POST $BASE/dbs/app/tables/notas/records/query -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"filter":{"titulo":{"$contains":"primeira"}}}'
```
