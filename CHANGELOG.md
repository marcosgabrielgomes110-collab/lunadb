# Changelog

All notable changes to this project are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Cursor (keyset) pagination** for listing and querying records: `next_cursor` in responses and a
  `cursor` parameter/field. Stable under concurrent inserts and deletes, unlike `offset`.
- Python SDK helpers `list_records_page`, `iter_records`, and `iter_query`; shell commands
  `list-next` and `walk`.
- Per-table and per-database read/write locks, so writes to different tables run in parallel
  while reads of a table are consistent and writes to the same table stay serialized.
- Portability: the `migrate` server-running check works on Linux, macOS, and Windows.

### Changed

- Reads of a table file now take a shared lock, so a response can no longer observe a partially
  written table.
- `create_db`/`delete_db` update the database registry under the metadata lock, closing a race
  between concurrent create and delete.

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

[Unreleased]: https://github.com/marcosgabrielgomes110-collab/lunadb/commits/main
