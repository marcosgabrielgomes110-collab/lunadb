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
| 429 | muitas tentativas de login (bloqueio temporário) | `{"error":"..."}` + header `Retry-After` |
| 500 | erro interno / falha de criptografia | `{"error":"..."}` |

O corpo de cada requisição é limitado a `[server] max_body_bytes` (default 10 MiB) e o
processamento a `request_timeout_secs` (default 30s → `408`).

## Sistema

### `GET /health` — público
```json
{"status":"ok","name":"lunadb","version":"0.0.1"}
```

### `GET /status` — admin
`pid`, `version`, `started_at`, `uptime_seconds`, `host`, `port`, `storage`,
`config`, `log_file`, `users`, `dbs`.

### `GET /metrics` — público
Contadores no formato de texto do Prometheus (`text/plain`): `lunadb_build_info`,
`lunadb_uptime_seconds`, `lunadb_requests_total` e `lunadb_responses_total{status_class="2xx|3xx|4xx|5xx"}`.
Apenas agregados, sem dados sensíveis.

## Auth e API keys

### `POST /auth/login` — público
```json
{"username":"root","password":"changeme"}
```
→ `200` `{"user":{...},"api_key":"luna_..."}`

Rotaciona **apenas** a chave de nome `login` do usuário; as demais continuam válidas.
Após 5 falhas consecutivas para o mesmo usuário, o login fica bloqueado por 60s
(retorna `429` com `Retry-After`); um login bem-sucedido zera o contador.

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

### `PUT /users/{id}/password`
```json
{"password":"novaSenha123","current_password":"antiga123"}
```
→ `UserPublic`. Admin pode trocar a senha de qualquer usuário (sem `current_password`);
o próprio usuário não-admin deve informar `current_password`. Mínimo de 6 caracteres.
A troca **revoga todas as API keys ativas** do usuário. Trocar a senha do root também
atualiza (e recifra) `root.password` no `config.toml`.

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
| GET | `/dbs/{db}/tables/{table}/records?limit=&offset=&cursor=` | read | `{"db","table","total","count","next_cursor","records":[{"_id":...}]}` |
| POST | `/dbs/{db}/tables/{table}/records` | write | `201` registro com `_id` |
| POST | `/dbs/{db}/tables/{table}/records/validate` | write | `200` registro preparado (dry-run, não persiste) |
| POST | `/dbs/{db}/tables/{table}/records/query` | read | `{"db","table","total","count","next_cursor","records"}` |
| POST | `/dbs/{db}/tables/{table}/records/aggregate` | read | `{"db","table","total","count","groups"}` |
| GET | `/dbs/{db}/tables/{table}/records/{id}` | read | registro |
| PUT | `/dbs/{db}/tables/{table}/records/{id}` | write | registro (substituição total) |
| PATCH | `/dbs/{db}/tables/{table}/records/{id}` | write | registro (merge parcial) |
| DELETE | `/dbs/{db}/tables/{table}/records/{id}` | write | `204` |

O corpo do registro deve ser um objeto JSON; `_id` é gerado pelo servidor e ignorado se enviado.
`total` é o tamanho da tabela e `count` o retornado na página; `limit` é limitado a 1000
(default), evitando respostas sem teto.

### Paginação por cursor (`next_cursor` / `cursor`)

Tanto a listagem quanto a query aceitam um `cursor` opaco e devolvem `next_cursor`. O cursor é uma
posição **keyset** codificada em base64url (os valores de ordenação do último registro, mais `_id`
como desempate único), não um offset inteiro. Consequências:

- Uma varredura nunca pula nem repete registro, mesmo com inserções ou remoções entre páginas.
- `total` é a contagem antes da janela do cursor, então permanece igual em todas as páginas.
- `_id` é sempre anexado como última chave de ordenação. Registros sem valor (ou nulo) no campo de
  ordenação continuam por último, e o desempate mantém a ordem total.
- Sem `sort`, os registros voltam ordenados por `_id` crescente.
- `next_cursor` só vem enquanto houver mais páginas; a última página (curta) o omite.
- `offset` continua funcionando e é aplicado na primeira página quando não há `cursor`. Enviar
  `cursor` e `offset` juntos usa o cursor e ignora o offset.
- Cursor malformado, ou que não bate com o `sort` atual, retorna `400`.

```bash
# primeira página
curl -s "$BASE/dbs/app/tables/notas/records?limit=50" -H "authorization: Bearer $KEY"
# -> {"total":1200,...,"next_cursor":"eyJrIjpbLi4uXX0"}

# próxima página
curl -s "$BASE/dbs/app/tables/notas/records?limit=50&cursor=eyJrIjpbLi4uXX0" \
  -H "authorization: Bearer $KEY"
```

O mesmo vale para `POST .../records/query`, que recebe `cursor` no corpo JSON.

### Merge parcial (`PATCH .../records/{id}`)

Aplica um [merge patch](https://datatracker.ietf.org/doc/html/rfc7386): objetos são mesclados
recursivamente, `null` remove a chave e demais valores substituem. O registro resultante passa
pela mesma validação/uniqueness do `PUT`, preservando campos `auto: created_at`.

```json
{"idade": 31, "endereco": {"cidade": "RJ"}, "telefone": null}
```

### Agregações (`POST .../records/aggregate`)

```json
{
  "filter": {"ativo": true},
  "group_by": ["cidade"],
  "aggregates": {
    "total": {"op": "count"},
    "receita": {"op": "sum", "field": "valor"},
    "ticket": {"op": "avg", "field": "valor"},
    "menor": {"op": "min", "field": "valor"},
    "maior": {"op": "max", "field": "valor"}
  },
  "limit": 50
}
```

Retorna `groups`, cada um com `key` (valores de `group_by`), `count` e os nomes de
`aggregates`. Operações: `count`, `sum`, `avg`, `min`, `max` (as numéricas exigem `field`).
`total` é o número de registros que passaram pelo filtro. Grupos ordenados pela chave.

### Query

```json
{
  "filter": {"$or": [{"idade": {"$gte": 18}}, {"tags": {"$contains": "vip"}}],
             "endereco.cidade": "SP"},
  "sort": ["-idade", {"field": "nome", "dir": "asc"}],
  "fields": ["nome", "endereco.cidade"],
  "limit": 20,
  "offset": 0,
  "cursor": "eyJrIjpbLi4uXX0"
}
```

- **Operadores**: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$exists`, `$contains` (substring em string / pertence em array), `$starts_with`, `$ends_with`, `$regex`, `$and`, `$or`, `$not`. Igualdade é implícita: `{"tenant": "t1"}`.
- **Caminhos**: `a.b` (objetos) e `tags.0` (índice de array). Filtro por `_id` funciona.
- **Sort**: `"campo"`, `"-campo"` (desc) ou `{"field","dir"}`; nulos/ausentes sempre por último. Padrão: `_id` crescente.
- **`fields`**: projeção (aceita caminhos aninhados); `_id` sempre incluído.
- **`total`**: total filtrado antes de `limit`/`offset`/`cursor`.
- **`cursor`/`next_cursor`**: paginação keyset; veja [Paginação por cursor](#paginação-por-cursor-next_cursor--cursor).
- Campos `encrypted` podem ser filtrados (decifra em memória).
- Campos desconhecidos no corpo → `422`; operador/filtro/cursor inválido → `400`.

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
