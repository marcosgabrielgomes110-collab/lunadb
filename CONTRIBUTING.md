# Contributing to LunaDB

Thanks for your interest in improving LunaDB. This document describes how to set up the
project, the expectations for changes, and the review process.

## Getting started

1. Install Rust 1.85+ (edition 2024) and, for the SDK, Python 3.11+.
2. Fork and clone the repository.
3. Build and run the test suite:

   ```bash
   make build
   make test
   ```

   or without `make`:

   ```bash
   cargo build
   cargo test --all
   PYTHONPATH=lunapy/src python -m pytest lunapy/tests
   ```

4. Create a topic branch: `git switch -c feat/short-description`.

## Development workflow

- `make fmt` — format the code with `rustfmt`.
- `make lint` — `cargo fmt --check` + `clippy` with `-D warnings`.
- `make test` — Rust unit/integration tests and the Python SDK tests.
- `make migrate TO=json` — exercise the offline storage-format migration.

Before opening a pull request, make sure `make lint` and `make test` pass.

## Guidelines

- Keep changes focused; small pull requests are easier to review.
- Match the existing code style, naming, and module structure.
- Public behavior (HTTP routes, JSON keys, the schema DSL, config keys, and the Python API)
  is part of the contract. Call out any intentional change to it in the pull request.
- Add tests for new behavior. Bug fixes should include a regression test when practical.
- Update the relevant document under `docs/` when you change user-facing behavior.
- Do not commit build artifacts, secrets, or the generated `~/.lunadb` data directory.

## Commit messages

Use clear, imperative subjects (for example, `Add compound unique indexes`). Reference an
issue in the body when relevant. Keep unrelated changes in separate commits.

## Reporting bugs and requesting features

Open a GitHub issue using the provided templates. Include your OS, the LunaDB version
(`lunadb --version`), and reproduction steps. For security issues, follow
[SECURITY.md](SECURITY.md) instead of opening a public issue.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
