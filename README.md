<p align="center">
  <img src="docs/assets/logo.png" alt="LunaDB Logo" width="220" />
</p>

# LunaDB

<p align="center">
  <a href="https://github.com/marcosgabrielgomes110-collab/lunadb/actions/workflows/ci.yml"><img src="https://github.com/marcosgabrielgomes110-collab/lunadb/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-edition%202024-orange.svg" alt="Rust" /></a>
  <a href="CHANGELOG.md"><img src="https://img.shields.io/badge/version-0.0.1-informational" alt="Version" /></a>
</p>

LunaDB is a self-contained JSON/MessagePack document database exposed as an HTTP API.
Each table is a single file on disk, records can be validated against an optional schema,
sensitive fields can be encrypted at rest, and access is controlled with roles, per-database
grants, and hashed API keys. It ships with a CLI/REPL and an official, dependency-free Python SDK.

> Status: **early (0.0.1)**. The API and on-disk format may change before 1.0.

## Features

- **HTTP API** built with [axum](https://github.com/tokio-rs/axum): databases, tables, schemas,
  records, queries, aggregations, users, roles, and API keys.
- **File storage** per table, in **JSON** or **MessagePack** (atomic writes via temp file + rename),
  with an offline format migration command.
- **Schema DSL**: typed fields (`string`, `int`, `float`, `bool`, `datetime`, `date`, `uuid`,
  `object`, `array`, `any`), constraints, enums, defaults, `required`/`nullable`, `strict` mode,
  `auto` timestamps, single/compound **unique** keys, and incremental migrations (`PUT`/`PATCH`).
- **Encryption at rest**: per-database AES-256-GCM keys derived from a master key with HKDF-SHA256.
- **AuthN/AuthZ**: argon2id password hashing, hashed API keys with revocation, roles and
  per-database grants (`read` < `write` < `admin`), login rate limiting.
- **Queries**: filters (`$eq`, `$gt`, `$in`, `$exists`, `$contains`, `$regex`, `$and`, `$or`, `$not`, ...),
  nested paths, multi-field sort, projection, pagination, and grouped aggregations
  (`count`/`sum`/`avg`/`min`/`max`).
- **Cursor pagination**: `next_cursor`/`cursor` keyset paging that is stable under concurrent writes,
  alongside the classic `limit`/`offset`.
- **Operations**: structured logging with rotation, Prometheus `/metrics`, request body limits
  and timeouts, graceful shutdown, and per-table read/write locking so writes to different
  tables run in parallel.
- **Tooling**: interactive shell (`lunadb shell`), non-interactive `apikey` commands, a `Makefile`,
  and a Python SDK (`lunapy`).

## Requirements

- Rust **1.85+** (edition 2024)
- Python **3.11+** (only for the SDK)
- Linux, macOS, or Windows

## Quickstart

Build and initialize:

```bash
cargo build --release
./target/release/lunadb init     # creates ~/.lunadb (config, roles, root, master key)
./target/release/lunadb serve    # starts the API on 127.0.0.1:7700
```

Log in and create a database, table, and record:

```bash
BASE=http://127.0.0.1:7700
KEY=$(curl -s -X POST $BASE/auth/login -H 'content-type: application/json' \
  -d '{"username":"root","password":"changeme"}' | jq -r .api_key)

curl -s -X POST $BASE/dbs -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"name":"app"}'

curl -s -X POST $BASE/dbs/app/tables -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"name":"notes"}'

curl -s -X POST $BASE/dbs/app/tables/notes/records -H "authorization: Bearer $KEY" \
  -H 'content-type: application/json' -d '{"title":"first note"}'
```

The default root password is `changeme`; change it in the config before exposing the server.
On first run the plaintext password in `config.toml` is replaced by an argon2id hash.

## CLI

```bash
lunadb init       # bootstrap config, roles, root user, and master key, then exit
lunadb serve      # run the API (default command)
lunadb shell      # interactive REPL
lunadb apikey ... # create/list/revoke API keys (scriptable)
lunadb migrate    # convert tables between msgpack and json (server stopped)
```

## Python SDK

```bash
pip install -e lunapy          # from this repository
pip install -e "lunapy[dev]"   # with pytest
```

```python
from lunapy import connect

client = connect(base_url="http://127.0.0.1:7700", username="root", password="changeme")
client.create_db("app")

notes = client.db("app").create_table("notes")  # Table handle
notes.create(title="hello")                     # kwargs become record fields
print(notes.where(title__contains="hell").count())

# the low-level API keeps working
client.create_record("app", "notes", {"title": "hello"})
```

## Development

The repository is driven by a `Makefile`:

```bash
make help        # list targets
make build       # debug build
make test        # Rust + Python test suites
make lint        # rustfmt check + clippy
make luna-setup  # interactive config.toml generator
make run         # cargo run -- serve
```

`make luna-setup` prompts for the main settings and writes a `config.toml`; pressing Enter
accepts the default shown in brackets, and Enter on the password generates a random one:

```bash
make luna-setup                                   # writes ~/.lunadb/config.toml
make luna-setup ARGS="--config /tmp/app/config.toml --force"
```

Manual equivalents:

```bash
cargo test --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
PYTHONPATH=lunapy/src python -m pytest lunapy/tests
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development workflow.

## Docker

Build and run with Docker:

```bash
docker build -t lunadb .
docker run -d --name lunadb -p 7700:7700 -v lunadb-data:/var/lib/lunadb lunadb
```

Or use Docker Compose:

```bash
docker compose up -d       # start
docker compose logs -f     # follow logs
docker compose down        # stop (data is preserved in the volume)
```

The container initialises config, master key, and root user on first boot. Mount
a custom `config.toml` at `/etc/lunadb/config.toml` to override defaults.

## Systemd

A production-hardened service unit is provided in `contrib/lunadb.service`:

```bash
sudo useradd --system --create-home --home-dir /var/lib/lunadb lunadb
sudo install -m 0755 target/release/lunadb /usr/local/bin/lunadb
sudo install -d -m 0750 -o lunadb -g lunadb /etc/lunadb /var/lib/lunadb
sudo cp contrib/lunadb.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now lunadb
```

Check status and logs:

```bash
sudo systemctl status lunadb
journalctl -u lunadb -f
```

## Documentation

- [HTTP API](docs/api.md)
- [Shell and CLI](docs/shell.md)
- [Python SDK](docs/sdk.md)
- [Operations](docs/operations.md) — layout on disk, backup, logs, capacity and known
  limitations, and troubleshooting

Portuguese versions are available alongside each document (for example
[`docs/api-pt.md`](docs/api-pt.md)).

## Project layout

```
src/                 Rust library and binary (API, engine, schema, storage, crypto)
lunapy/              Python SDK
docs/                API, CLI, and SDK documentation
tests/               Rust integration tests
contrib/             Deployment files (systemd service unit)
Dockerfile           Multi-stage production container image
docker-compose.yml   Single-container Compose deployment
Makefile             build/test/run/install shortcuts
```

## Security

Please report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
Never run `lunadb serve` with the default password on an untrusted network.

## License

Licensed under the [MIT License](LICENSE).
