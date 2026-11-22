# Contributing to Djangors

Thank you for your interest in contributing to Djangors!

## Development Setup

- **Rust toolchain**: see `rust-version` in [Cargo.toml](Cargo.toml) (MSRV is CI-enforced).
- **Test runner**: we recommend [`cargo-nextest`](https://nexte.st/) (`cargo install cargo-nextest`).
- **Fast local tests (SQLite, ~15s)**:
  ```bash
  TEST_BACKEND=sqlite cargo nextest run --workspace
  ```
- **Full suite with PostgreSQL 16**:
  ```bash
  docker run -d --rm --name djangors-pg -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=djangors_test -p 5432:5432 postgres:16-alpine
  DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/djangors_test cargo nextest run --workspace
  ```

## Quality Gates

Before opening a pull request, please ensure all of the following pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
TEST_BACKEND=sqlite cargo nextest run --workspace
```

All workspace crates enforce `#![deny(missing_docs)]`. Please document every new public item
and include unit or integration tests covering your change.

## Pull Requests & RFCs

- Small bug fixes and documentation improvements can go straight to a PR.
- Medium or larger public API changes should include a short design doc in `docs/design/`
  explaining the motivation and API shape (see `docs/src/api-stability.md`).
- Keep commits focused and descriptive.
- Update `CHANGELOG.md` under `[Unreleased]` for user-visible changes.

## Code of Conduct

All contributors are expected to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).
