# Operations

> See also: [HTTP API](api.md) · [Shell and CLI](shell.md) · [Python SDK](sdk.md)

This document covers running LunaDB in practice: where state lives, how to back it up, how to
read the logs, and what to do when something goes wrong. It describes behaviour of the code as
it is today (0.0.1); the on-disk format may still change before 1.0.

## On-disk layout

Everything lives under `storage.path` from the config (`~/.lunadb` by default):

```
~/.lunadb/
├── config.toml              server, storage, root and logging settings (mode 600)
├── meta/
│   ├── users.json           users, password hashes, API key hashes
│   ├── roles.json           roles and their permissions
│   ├── dbs.json             database registry
│   └── master.key           the encryption master key (mode 600)
├── dbs/
│   └── <db>/
│       └── <table>.msgpack  one file per table (or .json if format = "json")
└── infra/
    ├── status.json          pid/port/version of the running server
    ├── shell_history        REPL history
    └── logs/
        └── lunadb.log.*     rotated logs (see logging.rotation)
```

Two files matter more than the rest:

- **`meta/master.key`** derives the per-database encryption keys. **If it is lost, every
  `encrypted` field becomes unreadable.** It is not derived from anything else and cannot be
  recovered.
- **`meta/users.json`** holds the argon2id password hashes and the hashed API keys. Losing it
  means losing every credential, but not the data.

## Backup

The server has no built-in backup, and `init` does not create one. A safe backup is a copy of
the storage directory taken **while the server is stopped**, so no table is mid-write:

```bash
# stop the server first (Ctrl+C or SIGTERM), then:
tar czf lunadb-backup-$(date +%F).tar.gz -C ~ .lunadb
```

Restoring is the reverse: stop the server, restore the directory, start it again. Because
writes are atomic (temp file + rename) a live copy is unlikely to be corrupt, but it can still
catch a table between two changes, so a stopped copy is what you should rely on.

The master key is part of the backup and is required to read encrypted fields. Keep it in a
separate, safe place as well; storing the only copy next to the data defeats its purpose.

## Capacity and known limitations

LunaDB stores **one file per table** and keeps the whole table in memory while a request runs.
Every mutation reads the file, changes it, and writes it back atomically, so the cost of a write
grows with the size of the table:

- **A single record write rewrites the whole table.** Inserting one record into a table of 100k
  records encodes and fsyncs all 100k entries. Writes stay correct and safe, but they do not
  scale linearly.
- **Writes to the same table are serialized** (per-table write lock). Writes to different tables
  run in parallel.
- **Unique indexes are rebuilt per write.** When a schema has `unique` fields, the index is
  scanned from every record on each insert/update, which adds another full pass over the table.
- **Reads decode the whole table** and then apply the filter, sort, and pagination in memory.
  `limit`/`cursor` bound the response size, not the work done.

Practical guidance, as of 0.0.1:

| Table size | Behaviour |
|---|---|
| up to ~10k records | comfortable for interactive use |
| ~10k–100k records | every write re-encodes the file; batch work into fewer, larger requests where possible |
| above ~100k records | not recommended; split the data across several tables |

There are **no multi-table transactions** and no secondary indexes other than `unique` — an
operation touches exactly one table file, so cross-table consistency is the client's job.
Because the format is pre-1.0, treat these numbers as operational advice rather than a contract;
measure with your own record sizes before relying on them.

## Health, status, and metrics

| Endpoint | Auth | Purpose |
|---|---|---|
| `GET /health` | public | liveness only: `{"status":"ok","name":"lunadb","version":"..."}` |
| `GET /status` | admin | `pid`, `version`, `started_at`, `uptime_seconds`, `host`, `port`, `storage`, `config`, `log_file`, `users`, `dbs` |
| `GET /metrics` | public | Prometheus counters (see below) |

`/health` does not touch the storage layer, so it staying green does not by itself prove the
data is readable — it proves the process is up and serving.

`/metrics` exposes `lunadb_build_info`, `lunadb_uptime_seconds`, `lunadb_requests_total`, and
`lunadb_responses_total{status_class="2xx|3xx|4xx|5xx"}`. A rising `4xx`/`5xx` ratio is the
first thing to look at; the counters are process-wide and reset when the server restarts.

## Logs

Logging is configured under `[logging]` in `config.toml`:

```toml
[logging]
level = "info"      # trace | debug | info | warn | error
console = true      # also write to stdout
file = true         # write to infra/logs/
rotation = "daily"  # never | hourly | daily
max_files = 7       # how many rotated files to keep
```

`RUST_LOG` overrides the level when set, for example `RUST_LOG=lunadb=debug lunadb serve`.

Two kinds of lines matter when debugging:

- **`lunadb::ops`** entries are one per operation, with `op`, `user`, `db`, `table`, and a
  `count`/`id`. Grep for the operation name: `grep 'op="create_record"' lunadb.log`.
- **`http_request`** entries carry `method`, `uri`, `status`, and `latency_ms`. Client errors log
  at `warn`, server errors at `error`.

Sensitive values are never logged: no passwords, no API keys, and encrypted fields stay
encrypted in the table file.

## Common problems

### `error: api key missing or invalid` (401)

The key is absent, revoked, or belongs to a rotated login. `POST /auth/login` rotates only the
key named `login`; a key created with `lunadb apikey create` keeps working until revoked. If you
changed a password, **all** of that user's keys were revoked and a new login is required.

### `too many attempts; try again in Ns` (429)

After 5 consecutive failures for the same username, login is blocked for 60 seconds. A
successful login resets the counter. The lock is per username, so another account is unaffected.

### `no access to db '<name>'` (403)

The user has no grant covering that database. Check the effective level, which is the **max** of
the user's own grants and the permissions of their role, with `*` matching any database:

```bash
curl -s $BASE/me -H "authorization: Bearer $KEY"        # own grants and role
curl -s $BASE/roles -H "authorization: Bearer $KEY"     # role permissions (admin)
```

### `option 'admin' required on db '<name>'` when deleting a database

Only an admin (or a grant of `admin` on that database) can delete it. `admin` is strictly
stronger than `write`, so a `write` grant is not enough.

### `schema rejected by existing records` (409)

A schema change would invalidate records; the response lists the offending ids and fields. Fix
the records first, or repeat the request with `force=true` to keep the invalid records as they
are. For example through the shell:

```bash
schema set app users '<json>' --force
```

### `failed to decrypt field '<name>'` (500)

The stored value cannot be decrypted with the current master key. Causes, in order of
likelihood: a different `LUNADB_MASTER_KEY` is set than the one used to write the data, the key
file was replaced, or the key moved between databases. **The data is not recoverable without the
original key.** Restore `meta/master.key` from backup; do not delete the field to "fix" it,
since that loses data that a restored key could still read.

### `server appears to be running` on `lunadb migrate`

`migrate` refuses to run while the server holds the storage, because it rewrites every table
file. Stop the server and retry, or pass `--force` if you are certain. The check reads
`infra/status.json` and verifies the recorded pid is still alive, so a leftover file from a
crashed server will not block you.

### The server does not start

- **`Cannot assign requested address (os error 99)`** — `server.host` is an address that does
  not exist on this machine. Use `127.0.0.1` or `0.0.0.0`.
- **`Address already in use`** — another process holds the port; check `ss -ltnp | grep 7700`.
- **`failed to read master key at ...`** — the storage directory is wrong or incomplete. Run
  `lunadb init`, or point `storage.path` at the right directory.

### A table file looks corrupted

Table writes are atomic (write to a temp file, then rename), so a torn file is unlikely. If a
table fails to parse, the file can be inspected directly: `msgpack` files are MessagePack,
`json` files are pretty-printed JSON. Restore that single file from a backup while the server is
stopped. `lunadb migrate` can also convert a readable table between formats offline.

## Upgrading

The project is pre-1.0 and the on-disk format may change. Before upgrading:

1. Stop the server and take a backup (above), including `meta/master.key`.
2. Build or install the new binary.
3. Delete the old `infra/status.json` if it is stale — it is recreated at startup.
4. Start the server and check `GET /health`, then log in and read a table.

There are no automatic migrations of the storage layout beyond `lunadb migrate`, which only
converts a table between JSON and MessagePack.
