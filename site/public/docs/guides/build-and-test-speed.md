# Build, Test & Disk Size Optimization

Rust's compiler gives you small binaries and fast runtime performance, but Cargo's default settings optimize for debugging every third-party crate—which can leave `target/` at 13+ GiB, release binaries at 40+ MiB, and workspace checks feeling sluggish.

This guide documents every optimization Djangors ships out of the box (including `dj new`) and the workflow tricks that keep inner-loop checks, tests, CI, and disk usage fast and small.

## Measured Results in Djangors

| Metric | Before | After |
|---|---|---|
| Minimal Postgres release binary (`examples/minimal-facade`) | — | **4.00 MiB** stripped |
| Full-stack release binary (`examples/polls`) | — | **11.51 MiB** stripped |
| `target/` disk footprint (workspace) | 9.2 GiB – 13+ GiB | **3.6 GiB** (hard-capped at **4 GiB**) |
| Parallel SQLite workspace suite (`747 tests`) | `14.59s` | **`4.26s`** (~3.4x faster) |
| Core DB/auth/admin/REST loop (`179 tests`) | Serialized | **`1.01s`** |
| Real-socket SSE broadcast test | Flaky / hang risk | **`0.010s`** |

---

## 1. Small `target/` & Go-Sized Release Binaries

`dj new` and the Djangors workspace both configure `[profile.dev]` and `[profile.release]` in `Cargo.toml`:

```toml
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = 0

[profile.release]
opt-level = "z"
lto = "thin"
strip = true
codegen-units = 1
panic = "abort"
```

- **`debug = "line-tables-only"`** keeps file + line numbers in panic backtraces while cutting debug info size by ~65–70%.
- **`[profile.dev.package."*"] debug = 0`** omits DWARF debug info for third-party dependencies (which rarely need step-through debugging).
- **Feature-gate `djangors`**: always use `default-features = false` and enable only the batteries you use (`features = ["postgres", "rest", "auth"]`) so unused crates are never compiled or linked.

---

## 2. Avoid the `[profile.test]` vs `[profile.dev]` Double-Compilation Trap

Never set `debug` or `opt-level` on `[profile.test]` unless it matches `[profile.dev]` identically.

If `[profile.dev]` uses `debug = "line-tables-only"` and `[profile.test]` overrides `debug = 0` (or `opt-level = 1`), Cargo treats them as incompatible compiler fingerprints and compiles **every dependency in your workspace twice**—doubling both `target/debug/deps/` size and test build times.

Keep `[profile.test]` untouched so it inherits `[profile.dev]` directly, and tune only specific leaf crates via `[profile.dev.package.<crate>]`.

---

## 3. Selective `opt-level = 3` for Crypto & Hashing Crates

Pure-Rust cryptography and password hashing are 30–150x slower at `opt-level = 0`. A single Argon2 hash or 2048-bit RSA signature in an unoptimized test can take seconds.

Compile just those leaf crates with full optimization in dev/test builds:

```toml
[profile.dev.package.argon2]
opt-level = 3
[profile.dev.package.blake2]
opt-level = 3
[profile.dev.package.rsa]
opt-level = 3
[profile.dev.package.sha2]
opt-level = 3
[profile.dev.package.hmac]
opt-level = 3
[profile.dev.package.aes]
opt-level = 3
[profile.dev.package.aes-gcm]
opt-level = 3
[profile.dev.package.ghash]
opt-level = 3
[profile.dev.package.p256]
opt-level = 3
[profile.dev.package.p384]
opt-level = 3
[profile.dev.package.curve25519-dalek]
opt-level = 3
[profile.dev.package.ed25519-dalek]
opt-level = 3
[profile.dev.package.zxcvbn]
opt-level = 3
```

Because these are leaf crates that rarely change, Cargo compiles them once and your workspace crates still rebuild incrementally at `opt-level = 0`.

---

## 4. Eliminate Dead Test Binaries & Slim `trybuild`

By default, Cargo compiles a separate `--test` harness executable for every `[lib]` and `[[bin]]` target—even if it contains zero `#[test]` functions. In a multi-crate workspace, each empty executable links 50–60 MiB of dependencies.

For binary-only or example crates whose tests live in `tests/*.rs`:

```toml
[lib]
test = false
doctest = false

[[bin]]
name = "my_app"
test = false
```

Similarly, if you use `trybuild` for proc-macro UI tests, `trybuild` spawns an isolated inner Cargo build inside `target/tests/trybuild/`. Pin its `[dev-dependencies]` with `default-features = false` and only the single backend/feature those UI snippets need so `trybuild` does not compile C SQLite (`libsqlite3-sys`) or optional stacks inside a second `target/` tree.

---

## 5. Fast Parallel `cargo-nextest` & Race-Free Async Tests

### Separate Postgres serialization from SQLite parallelism

Shared PostgreSQL test databases require serialized access per table (`max-threads = 1` in `.config/nextest.toml`). However, SQLite tests in Djangors use isolated in-memory `sqlite::memory:` connections and can run across every CPU core.

`.config/nextest.toml` defines a `sqlite` profile that promotes DB crates back to `@global` parallelism while keeping nested-`cargo` subprocess tests serialized, and sets `terminate-after` so a hung test never stalls CI or your terminal forever:

```toml
[profile.default]
slow-timeout = { period = "15s", terminate-after = 4 }

[test-groups]
cargo-subprocess = { max-threads = 1 }
postgres-db = { max-threads = 1 }

[[profile.sqlite.overrides]]
filter = "package(djangors-orm) or package(djangors-auth) or package(djangors-admin)"
test-group = "@global"
```

Use the workspace Cargo aliases in `.cargo/config.toml`:

```bash
# Fast single-crate type/borrow check (skips codegen and linking)
cargo cf -p djangors-orm

# Full parallel SQLite suite
cargo t

# Instant ~4s inner-loop suite (747 tests; skips Postgres-only crates & nested-cargo builds)
cargo t-fast
```

### Replace fixed `sleep()` calls with `tokio::sync::Notify` + `timeout`

Under high parallel CPU load, `tokio::time::sleep(Duration::from_millis(50))` frequently wakes up *before* a spawned server or SSE handler has subscribed, causing flaky failures or infinite `read()` hangs.

Always synchronize with `tokio::sync::Notify` and wrap socket reads in `tokio::time::timeout`:

```rust,illustrative
let subscribed = std::sync::Arc::new(tokio::sync::Notify::new());
// Inside the handler after subscribing: subscribed.notify_one();

tokio::time::timeout(std::time::Duration::from_secs(5), subscribed.notified())
    .await
    .expect("handler should subscribe before publish");
```

---

## 6. Making `cargo check` & Your Editor Instant

`cargo check` (`rustc --emit=metadata`) skips LLVM codegen and linking, but in large workspaces it can still feel slow for one main reason: **feature-unification hash misses**.

- `cargo check -p djangors-orm --lib` resolves dependencies with **Hash A**.
- `cargo check --workspace --lib` resolves them with **Hash B**.
- `cargo test` resolves them with **Hash C** (`dev-dependencies` included).
- `rust-analyzer` running `--workspace --all-targets` resolves **Hash D** and contends on `target/debug/.cargo-lock`.

Every time you switch between them, Cargo re-checks shared dependencies (`sqlx`, `tokio`, `rustls`). Fix this with four tools:

### 1. Scope `rust-analyzer` to the crate you are editing

Djangors includes `.zed/settings.json` so Zed checks only the active crate (`workspace = false`, `allTargets = false`) in its own `target/rust-analyzer` directory (`targetDir = true`) without blocking terminal `cargo`:

```json
{
  "lsp": {
    "rust-analyzer": {
      "initialization_options": {
        "check": {
          "workspace": false,
          "allTargets": false
        },
        "cargo": {
          "targetDir": true
        },
        "cachePriming": {
          "enable": false
        }
      }
    }
  }
}
```

*(In VS Code, set `"rust-analyzer.check.workspace": false`, `"rust-analyzer.check.allTargets": false`, and `"rust-analyzer.cargo.targetDir": true`.)*

### 2. Enable `sccache` in your user-level `~/.cargo/config.toml`

`sccache` caches compiled crates (`.rlib`, `.rmeta`, proc-macros) across feature hashes, branches, and `cargo clean`. Put this in **`~/.cargo/config.toml`** (not the project's `.cargo/config.toml`, so contributors without `sccache` aren't affected):

```toml
[build]
rustc-wrapper = "sccache"

[env]
SCCACHE_CACHE_SIZE = "4G"
```

### 3. Live background checks with `bacon`

Run `bacon check -p my_crate` in a side terminal. It watches file saves and runs `cargo check` continuously so diagnostics are ready before you switch windows.

### 4. Workspace-hack (`cargo-hakari`), Cranelift, and the LLD linker

- **`cargo-hakari`** generates a shared `workspace-hack` crate that unifies dependency features across all workspace members into one hash—eliminating re-checks when switching `-p <crate>` vs `--workspace`.
- **Nightly Cranelift** (`-Zcodegen-backend=cranelift`) speeds up unoptimized dev/test codegen by ~25–40% when iterating on tests.
- **Don't blindly add custom `RUSTFLAGS` for `mold`/`lld` on modern Linux `rustc`**: `rustc` 1.90+ on `x86_64-unknown-linux-gnu` already links with `rust-lld` by default, and custom `RUSTFLAGS` invalidate caches whenever an editor or tool invokes Cargo without matching flags.

---

## 7. Faster, Smaller CI

In CI (see `.circleci/config.yml`), set:

```yaml
environment:
  CARGO_INCREMENTAL: "0"
```

`target/debug/incremental/` can easily grow to 2.4+ GiB, bloats CI cache restore/save steps, and forces extra LLVM codegen units. Disabling incremental compilation in CI shrinks caches and speeds up clean CI builds.

---

## 8. Hard-Capping `target/` Disk Size

On stable Rust, `target/` is append-only (even nightly `cargo -Z gc` only sweeps `~/.cargo` global caches, not per-project `target/`).

Djangors includes [`scripts/trim-target.sh`](https://github.com/QUANTORA-ltd/djangors/blob/main/scripts/trim-target.sh) to enforce a hard size cap (default `4096 MiB` / 4 GiB) safely:

1. **Build-safety guards**: checks `flock -n` on `target/{debug,release}/.cargo-lock` and active `cargo`/`rustc`/`cargo-nextest` processes—exiting cleanly if a build is in progress.
2. **Staged pruning** (stops as soon as `target/` is under the cap):
   - Ephemeral dirs (`target/doc`, `target/tmp`, `target/flycheck*`)
   - Nested test sub-targets (`target/tests/trybuild`, `target/cli-test-cache`, then `target/rust-analyzer`)
   - Old incremental sessions, then `target/*/incremental` (keeps `.rlib`/`.rmeta` warm)
   - Stale duplicate hashed artifacts in `deps/` (keeps the 2 newest hashes per crate so `check` and `test` stay warm)
   - LRU eviction by `mtime` (preserves top-level release binaries like `target/release/dj`)

Run it manually or via a `systemd --user` timer every 15 minutes:

```bash
./scripts/trim-target.sh 4096
```

> **Tip:** See [Deployment](deployment.md#keeping-binaries-and-target-small) for production Docker multi-stage builds and `dj check --deploy`.
