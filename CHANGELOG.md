# Changelog

All notable changes to this project are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Cursor (keyset) pagination** for listing and querying records: `next_cursor` in responses and
  a `cursor` parameter/field. Stable under concurrent inserts and deletes, unlike `offset`.
- Python SDK helpers `list_records_page`, `iter_records`, and `iter_query`; shell commands
  `list-next` and `walk`.
- Per-table and per-database read/write locks, so writes to different tables run in parallel
  while reads of a table are consistent and writes to the same table stay serialized.
- Portability: the `migrate` server-running check works on Linux, macOS, and Windows.
- `make luna-setup` (and `scripts/luna-setup.sh`): interactive `config.toml` generator with
  defaults on Enter, a random root password option, and a `600` file mode.
- `scripts/test-luna-setup.sh` and `make test-setup`: end-to-end smoke test for the setup
  script, also run by CI under `dash` and `sh`.
- [Operations guide](docs/operations.md): on-disk layout, backup, logs, and troubleshooting
  (with a Portuguese version), now including a capacity and known-limitations section.
- CI: a dependency audit job (`cargo audit` and `pip-audit`) that fails on known
  vulnerabilities.
- Python SDK ergonomics, without breaking the existing API: `client.db()`/`client.table()`
  handles, chainable `where()`/`sort()`/`select()` queries, `connect()` and context-manager
  support, record creation/update with keyword arguments, `Record` dicts with attribute access,
  `Field.string()`/`Field.int()` shortcuts, and lenient schema inputs (`TableSchema(unique="email")`,
  string field definitions).

### Changed

- Table, metadata, and status writes now fsync the temporary file and the parent directory before returning, so an acknowledged write is durable across a crash and the behavior matches the operations guide.
- Dependencies: `base64` 0.23, `tower-http` 0.7, `actions/checkout` v7, `actions/setup-python`
  v7, and `pytest>=9.1.1`. The Python SDK builds with `setuptools>=84`.
- Transitive TLS dependencies updated to address RUSTSEC-2026-0346/0347/0348/0349/0381/0382:
  `rustls` 0.23.46, `rustls-webpki` 0.103.16, and `webpki-roots` 1.0.10.
- Reads of a table file now take a shared lock, so a response can no longer observe a partially
  written table.
- `create_db`/`delete_db` update the database registry under the metadata lock, closing a race
  between concurrent create and delete.
- Dependabot groups minor and patch updates per ecosystem, so a weekly run opens far fewer PRs.

### Fixed

- `GET .../records` applies `offset` before `limit`, so `limit=10&offset=10` returns the second
  page instead of an empty one; the same rule still holds when a cursor is supplied (the cursor
  wins and the offset is ignored).
- `next_cursor` is omitted when no records remain, avoiding an extra empty page at the end of a
  walk when the remaining records fit exactly in the page size.
- The dependency audit job installs `pip-audit` with `pip` instead of `cargo`, which has no such
  crate; the job previously failed before running either audit.
- The CHANGELOG listed the 0.0.1 feature set under `Unreleased` and repeated the `Changed`
  heading; the 0.0.1 entries are now under their own release.

## [0.0.1] - 2026-10-09

First public release.

### Added

- HTTP API for databases, tables, schemas, records, and queries, built with `axum`.
- Schema DSL with typed fields, constraints, enums, defaults, `strict` mode, `auto`
  timestamps, and single/compound `unique` keys.
- Schema migration via `PUT` (full) and `PATCH` (incremental) with `force` support.
- Query engine with filters, nested paths, sorting, projection, and pagination.
- Grouped aggregations (`count`/`sum`/`avg`/`min`/`max`).
- Partial record updates (`PATCH`) using merge-patch semantics.
- Per-database encryption at rest (AES-256-GCM with HKDF-SHA256 key derivation).
- Authentication with argon2id passwords and hashed, revocable API keys.
- Roles and per-database grants (`read`/`write`/`admin`).
- Login rate limiting and a password-change endpoint.
- JSON and MessagePack table storage with an offline migration command.
- Structured logging with rotation, Prometheus `/metrics`, request body limits, and timeouts.
- Interactive shell and non-interactive `apikey` CLI commands.
- Official Python SDK (`lunapy`) with typed models and errors.
- `Makefile` with build, test, lint, run, migrate, and install targets.

[Unreleased]: https://github.com/marcosgabrielgomes110-collab/lunadb/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/marcosgabrielgomes110-collab/lunadb/releases/tag/v0.0.1
