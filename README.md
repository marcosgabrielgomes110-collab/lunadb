# LunaDB

[![CI](https://github.com/marcosgabrielgomes110-collab/lunadb/actions/workflows/ci.yml/badge.svg)](https://github.com/marcosgabrielgomes110-collab/lunadb/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.0.1-informational)](CHANGELOG.md)

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
from lunapy import LunaClient

client = LunaClient(base_url="http://127.0.0.1:7700", api_key="luna_...")
client.login("root", "changeme")
client.create_db("app")
client.create_record("app", "notes", {"title": "hello"})
```

## Development

The repository is driven by a `Makefile`:

```bash
make help        # list targets
make build       # debug build
make test        # Rust + Python test suites
make lint        # rustfmt check + clippy
make run         # cargo run -- serve
```

Manual equivalents:

```bash
cargo test --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
PYTHONPATH=lunapy/src python -m pytest lunapy/tests
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development workflow.

## Documentation

- [HTTP API](docs/api.md)
- [Shell and CLI](docs/shell.md)
- [Python SDK](docs/sdk.md)

Portuguese versions are available alongside each document (for example
[`docs/api-pt.md`](docs/api-pt.md)).

## Project layout

```
src/                 Rust library and binary (API, engine, schema, storage, crypto)
lunapy/              Python SDK
docs/                API, CLI, and SDK documentation
tests/               Rust integration tests
Makefile             build/test/run/install shortcuts
```

## Security

Please report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
Never run `lunadb serve` with the default password on an untrusted network.

## License

Licensed under the [MIT License](LICENSE).
