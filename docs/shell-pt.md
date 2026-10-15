# Shell e CLI

> Veja também: [API HTTP](api.md) · [SDK Python](sdk.md)

O binário `lunadb` tem dois usos:

1. **Servidor**: `lunadb serve` (default), `lunadb init`, `lunadb migrate`.
2. **Cliente**: `lunadb shell` (REPL) e `lunadb apikey ...` (script/CI).

## Flags globais

| Flag | Descrição |
|---|---|
| `--config <path>` | caminho do `config.toml` (default `~/.lunadb/config.toml`; env `LUNADB_CONFIG`) |
| `--server <url>` | URL da API para comandos cliente (default: `http://{host}:{port}` do config) |
| `--key <key>` | API key para comandos cliente (env `LUNADB_API_KEY`) |

Variáveis de ambiente: `LUNADB_CONFIG`, `LUNADB_API_KEY`, `LUNADB_MASTER_KEY`, `RUST_LOG`.

## Servidor

```bash
lunadb init      # cria ~/.lunadb (config, roles, root, master key) e sai
lunadb serve     # sobe a API (Ctrl+C/SIGTERM encerram de forma graciosa)
```

## Setup interativo

```bash
make luna-setup                                  # grava ~/.lunadb/config.toml
make luna-setup ARGS="--config /tmp/app/config.toml"
make luna-setup ARGS="--force"                   # pula a confirmação de sobrescrita
```

Pergunta `host`, `port`, `storage.path`, `storage.format`, limite de corpo, timeout da requisição,
`username` e `password` do root e as opções de log. Enter aceita o default mostrado entre
colchetes; Enter na senha gera uma aleatória e a imprime uma única vez. O arquivo é gravado com
modo `600` e não sobrescreve uma config existente sem confirmação (ou `--force`). A senha fica em
texto puro até o primeiro boot convertê-la para argon2id.

## Comandos de API key (não-interativos)

```bash
export LUNADB_API_KEY=luna_...    # key de um admin

lunadb apikey create bob --name ci      # imprime a key nova (uma única vez)
lunadb apikey list bob                  # JSON das keys do bob
lunadb apikey list                      # JSON das próprias keys
lunadb apikey revoke bob luna_abc123    # por id ou prefixo
```

Saída de `create` é apenas a key (para `$()` em scripts); `list`/`revoke` retornam JSON.

## Migração de formato (offline)

Converte todas as tabelas entre MessagePack (default) e JSON. Exige servidor parado
(detecta via `infra/status.json` + pid; use `--force` para ignorar a checagem).

```bash
lunadb migrate --to json
lunadb migrate --to msgpack --force
```

## Shell interativo

```bash
lunadb shell
lunadb shell --server http://127.0.0.1:7700 --key luna_...
```

- Histórico persistido em `~/.lunadb/infra/shell_history`.
- `login` sem senha usa prompt oculto (rpassword).
- JSON sempre entre **aspas simples** (o parser remove as duplas):
  `add app notas '{"titulo":"oi"}'`.
- `exit`/`quit` (ou Ctrl+D) saem.

### Sessão e diagnóstico

| Comando | Exemplo |
|---|---|
| `login <user> [senha]` | `login root` (pede a senha) |
| `whoami` | retorna o `UserPublic` atual |
| `status` | pid, uptime, host/porta, contagens |
| `key rotate` | rotaciona a chave `login` e exibe a nova |
| `help` | lista os comandos |

### Databases e tabelas

| Comando | Exemplo |
|---|---|
| `dbs` | `dbs` |
| `db create <db>` | `db create app` |
| `db rm <db>` | `db rm app` |
| `tables <db>` | `tables app` |
| `table create <db> <t> [schema-json]` | `table create app users '{"fields":{"email":{"type":"string"}}}'` |
| `table rm <db> <t>` | `table rm app users` |

### Schema

| Comando | Exemplo |
|---|---|
| `schema get <db> <t>` | `schema get app users` |
| `schema set <db> <t> <json> [--force]` | `schema set app users '{"fields":{...}}' --force` |
| `schema patch <db> <t> <json> [--force]` | `schema patch app users '{"add":{"tel":{"type":"string"}}}'` |
| `schema add-field <db> <t> <nome> <json> [--force]` | `schema add-field app users tel '{"type":"string","default":""}'` |
| `schema rm-field <db> <t> <campo>... [--force]` | `schema rm-field app users antigo outro` |
| `schema update-field <db> <t> <campo> <json> [--force]` | `schema update-field app users email '{"required":true,"max":null}'` |
| `schema unique add <db> <t> <chave> [--force]` | `schema unique add app users tenant+email` |
| `schema unique rm <db> <t> <chave> [--force]` | `schema unique rm app users cpf` |
| `schema rm <db> <t>` | remove o schema inteiro |

Todas funcionam mesmo se a tabela ainda não tiver schema (criam do zero). Sem `--force`,
registros que violem o novo schema retornam `erro: HTTP 409 ...` com os ids.

### Registros e query

| Comando | Exemplo |
|---|---|
| `list <db> <t> [limit] [offset]` | `list app users 10 0` |
| `list-next <db> <t> <cursor> [limit]` | `list-next app users eyJrIjpbLi4uXX0 10` |
| `walk <db> <t> [page-size]` | `walk app users 200` (percorre todos os registros) |
| `get <db> <t> <id>` | `get app users 0f045bbf-...` |
| `add <db> <t> '<json>'` | `add app users '{"email":"a@x.com"}'` |
| `put <db> <t> <id> '<json>'` | `put app users <id> '{"email":"b@x.com"}'` |
| `patch <db> <t> <id> '<json>'` | `patch app users <id> '{"idade":31,"tel":null}'` |
| `rm <db> <t> <id>` | `rm app users <id>` |
| `query <db> <t> '<json>'` | `query app users '{"filter":{"idade":{"$gte":18}},"sort":["-idade"]}'` |
| `aggregate <db> <t> '<json>'` | `aggregate app users '{"group_by":["cidade"],"aggregates":{"n":{"op":"count"}}}'` |

### Usuários, grants e API keys

| Comando | Exemplo |
|---|---|
| `users` | lista usuários (admin) |
| `user add <user> <senha> [role] [db:level ...]` | `user add bob bob12345 user app:write` |
| `user rm <id>` | `user rm 3d717770-...` |
| `grants <user-id> '<json>'` | `grants <id> '[{"db":"app","level":"read"}]'` |
| `apikey create <user> [--name <nome>]` | `apikey create bob --name ci` |
| `apikey list [user]` | `apikey list bob` (sem user = próprias) |
| `apikey revoke <user> <id\|prefixo>` | `apikey revoke bob luna_abc123` |

### Exemplo de sessão

```
$ lunadb shell
LunaDB shell (http://127.0.0.1:7700) - 'help' para comandos, 'exit' para sair
lunadb> login root
senha de root:
logado como root
lunadb> db create app
db 'app' criado
lunadb> table create app notas '{"fields":{"titulo":{"type":"string","required":true}}}'
tabela 'app.notas' criada
lunadb> add app notas '{"titulo":"primeira nota"}'
{
  "_id": "0f045bbf-5a9a-43b5-81b4-46d9758c9721",
  "titulo": "primeira nota"
}
lunadb> query app notas '{"filter":{"titulo":{"$contains":"primeira"}},"fields":["titulo"]}'
{
  "count": 1,
  "db": "app",
  "records": [ { "_id": "...", "titulo": "primeira nota" } ],
  "table": "notas",
  "total": 1
}
lunadb> apikey create root --name console
api key criada (guarde agora): luna_4a7973...
lunadb> exit
```

Os erros aparecem como `erro: HTTP 403: ...`, sem encerrar a sessão.
