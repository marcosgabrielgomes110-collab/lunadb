# Shell and CLI

> See also: [HTTP API](api.md) · [Python SDK](sdk.md)

The `lunadb` binary has two uses:

1. **Server**: `lunadb serve` (default), `lunadb init`, `lunadb migrate`.
2. **Client**: `lunadb shell` (REPL) and `lunadb apikey ...` (script/CI).

## Global flags

| Flag | Description |
|---|---|
| `--config <path>` | path to `config.toml` (default `~/.lunadb/config.toml`; env `LUNADB_CONFIG`) |
| `--server <url>` | API URL for client commands (default: `http://{host}:{port}` from the config) |
| `--key <key>` | API key for client commands (env `LUNADB_API_KEY`) |

Environment variables: `LUNADB_CONFIG`, `LUNADB_API_KEY`, `LUNADB_MASTER_KEY`, `RUST_LOG`.

## Server

```bash
lunadb init      # creates ~/.lunadb (config, roles, root, master key) and exits
lunadb serve     # starts the API (Ctrl+C/SIGTERM shut down gracefully)
```

## API key commands (non-interactive)

```bash
export LUNADB_API_KEY=luna_...    # an admin's key

lunadb apikey create bob --name ci      # prints the new key (only once)
lunadb apikey list bob                  # JSON of bob's keys
lunadb apikey list                      # JSON of your own keys
lunadb apikey revoke bob luna_abc123    # by id or prefix
```

`create` output is just the key (for `$()` in scripts); `list`/`revoke` return JSON.

## Format migration (offline)

Converts all tables between MessagePack (default) and JSON. Requires the server stopped
(detected via `infra/status.json` + pid; use `--force` to skip the check).

```bash
lunadb migrate --to json
lunadb migrate --to msgpack --force
```

## Interactive shell

```bash
lunadb shell
lunadb shell --server http://127.0.0.1:7700 --key luna_...
```

- History persisted in `~/.lunadb/infra/shell_history`.
- `login` without a password uses a hidden prompt (rpassword).
- JSON always between **single quotes** (the parser strips the double quotes):
  `add app notes '{"title":"hi"}'`.
- `exit`/`quit` (or Ctrl+D) quit.

### Session and diagnostics

| Command | Example |
|---|---|
| `login <user> [password]` | `login root` (prompts for the password) |
| `whoami` | returns the current `UserPublic` |
| `status` | pid, uptime, host/port, counts |
| `key rotate` | rotates the `login` key and shows the new one |
| `help` | lists the commands |

### Databases and tables

| Command | Example |
|---|---|
| `dbs` | `dbs` |
| `db create <db>` | `db create app` |
| `db rm <db>` | `db rm app` |
| `tables <db>` | `tables app` |
| `table create <db> <t> [schema-json]` | `table create app users '{"fields":{"email":{"type":"string"}}}'` |
| `table rm <db> <t>` | `table rm app users` |

### Schema

| Command | Example |
|---|---|
| `schema get <db> <t>` | `schema get app users` |
| `schema set <db> <t> <json> [--force]` | `schema set app users '{"fields":{...}}' --force` |
| `schema patch <db> <t> <json> [--force]` | `schema patch app users '{"add":{"tel":{"type":"string"}}}'` |
| `schema add-field <db> <t> <name> <json> [--force]` | `schema add-field app users tel '{"type":"string","default":""}'` |
| `schema rm-field <db> <t> <field>... [--force]` | `schema rm-field app users old another` |
| `schema update-field <db> <t> <field> <json> [--force]` | `schema update-field app users email '{"required":true,"max":null}'` |
| `schema unique add <db> <t> <key> [--force]` | `schema unique add app users tenant+email` |
| `schema unique rm <db> <t> <key> [--force]` | `schema unique rm app users cpf` |
| `schema rm <db> <t>` | removes the entire schema |

All work even if the table has no schema yet (they create it from scratch). Without `--force`,
records that violate the new schema return `error: HTTP 409 ...` with the ids.

### Records and query

| Command | Example |
|---|---|
| `list <db> <t> [limit] [offset]` | `list app users 10 0` |
| `get <db> <t> <id>` | `get app users 0f045bbf-...` |
| `add <db> <t> '<json>'` | `add app users '{"email":"a@x.com"}'` |
| `put <db> <t> <id> '<json>'` | `put app users <id> '{"email":"b@x.com"}'` |
| `patch <db> <t> <id> '<json>'` | `patch app users <id> '{"age":31,"tel":null}'` |
| `rm <db> <t> <id>` | `rm app users <id>` |
| `query <db> <t> '<json>'` | `query app users '{"filter":{"age":{"$gte":18}},"sort":["-age"]}'` |
| `aggregate <db> <t> '<json>'` | `aggregate app users '{"group_by":["city"],"aggregates":{"n":{"op":"count"}}}'` |

### Users, grants, and API keys

| Command | Example |
|---|---|
| `users` | lists users (admin) |
| `user add <user> <password> [role] [db:level ...]` | `user add bob bob12345 user app:write` |
| `user rm <id>` | `user rm 3d717770-...` |
| `grants <user-id> '<json>'` | `grants <id> '[{"db":"app","level":"read"}]'` |
| `apikey create <user> [--name <name>]` | `apikey create bob --name ci` |
| `apikey list [user]` | `apikey list bob` (without user = your own) |
| `apikey revoke <user> <id\|prefix>` | `apikey revoke bob luna_abc123` |

### Example session

```
$ lunadb shell
LunaDB shell (http://127.0.0.1:7700) - 'help' for commands, 'exit' to quit
lunadb> login root
password for root:
logged in as root
lunadb> db create app
db 'app' created
lunadb> table create app notes '{"fields":{"title":{"type":"string","required":true}}}'
table 'app.notes' created
lunadb> add app notes '{"title":"first note"}'
{
  "_id": "0f045bbf-5a9a-43b5-81b4-46d9758c9721",
  "title": "first note"
}
lunadb> query app notes '{"filter":{"title":{"$contains":"first"}},"fields":["title"]}'
{
  "count": 1,
  "db": "app",
  "records": [ { "_id": "...", "title": "first note" } ],
  "table": "notes",
  "total": 1
}
lunadb> apikey create root --name console
api key created (save it now): luna_4a7973...
lunadb> exit
```

Errors appear as `error: HTTP 403: ...`, without ending the session.
