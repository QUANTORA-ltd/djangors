# The Rust Secrets Nobody Tells You: 4s Test Suites, 4 MiB Binaries, and a Hard-Capped `target/`

Most teams who try Rust for web backends love the runtime speed and type safety, then hit a wall: `target/` swells to 15 GiB, `cargo test` takes minutes, `cargo check` feels mysteriously sluggish in large workspaces, and release binaries ship at 50 MiB.

While shipping **Djangors 1.0** (a 38-crate workspace), we measured and fixed every one of these bottlenecks. Here are the real Rust and Cargo secrets that turned our workflow into:

| Metric | Before | After |
|---|---|---|
| Minimal Postgres release binary (`examples/minimal-facade`) | — | **4.00 MiB** stripped |
| Full-stack release binary (`examples/polls`) | — | **11.51 MiB** stripped |
| `target/` disk footprint | 9.2 GiB – 13+ GiB | **3.6 GiB** (hard-capped at **4 GiB**) |
| Parallel SQLite workspace suite (`747 tests`) | `14.59s` | **`4.26s`** (~3.4x faster) |
| Core DB/auth/admin/REST loop (`179 tests`) | Serialized | **`1.01s`** |
| Real-socket SSE broadcast test | Flaky / hang risk | **`0.010s`** |

---

## 1. The `[profile.test]` vs `[profile.dev]` Double-Compilation Trap

The single most expensive mistake in Rust `Cargo.toml` tuning is setting `debug = 0` or `opt-level = 1` on `[profile.test]` without matching `[profile.dev]`.

Cargo hashes every dependency's compiler flags into its artifact fingerprint. If `[profile.dev]` and `[profile.test]` disagree on `debug` or `opt-level`, Cargo compiles **every dependency in your workspace twice**—doubling `target/debug/deps/` size and doubling build times.

Keep `[profile.test]` untouched so it inherits `[profile.dev]`, and tune `[profile.dev]` directly:

```toml
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = 0
```

`debug = "line-tables-only"` preserves file + line numbers in panic backtraces while cutting debug symbol size by roughly two-thirds.

---

## 2. Selective `opt-level = 3` for Pure-Rust Crypto & Compression Leaves

Pure-Rust cryptography and password hashing are **30–150x slower** at `opt-level = 0`. A single Argon2 hash or 2048-bit RSA operation in an unoptimized test can burn seconds.

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

Because these are leaf crates that rarely change, Cargo compiles them once and your own workspace crates still rebuild incrementally at `opt-level = 0`.

---

## 3. Eliminate Dead Empty Test Binaries & Slim `trybuild`

By default, Cargo compiles a separate `--test` executable for every `[lib]` and `[[bin]]` target—even if it contains **zero** `#[test]` functions. In a multi-crate workspace, each empty test binary links 50–60 MiB of dependencies.

In our `examples/` crates (`polls`, `school`, `rest-api`, `minimal-facade`), unit tests didn't exist in `src/`—all tests lived in `tests/*.rs`. Setting:

```toml
[lib]
test = false
doctest = false

[[bin]]
name = "polls"
test = false
```

eliminated **~350 MiB** of dead test executables while keeping every integration test in `tests/*.rs` running normally.

Similarly, `trybuild` UI tests for proc-macros spawn a hidden second `target/` at `target/tests/trybuild/`. Pinning `trybuild`'s `[dev-dependencies]` to `default-features = false, features = ["postgres"]` stopped `trybuild` from compiling C SQLite (`libsqlite3-sys`) inside that nested target.

---

## 4. Parallel `cargo-nextest` Profiles & Race-Free Async Tests

Shared PostgreSQL test databases need serialized access per table (`max-threads = 1` in `.config/nextest.toml`). SQLite tests, however, use isolated in-memory `sqlite::memory:` connections and can run across every CPU core.

We added `[profile.sqlite]` in `.config/nextest.toml` to promote DB crates back to `@global` parallelism while keeping nested-`cargo` subprocess tests serialized, and added `terminate-after` so a hung test never stalls forever:

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

Combined with `.cargo/config.toml` aliases (`cargo t` and `cargo t-fast`), our warm 747-test parallel SQLite suite dropped from **`14.59s` to `4.261s`**, and our 179-test core DB/auth/admin/REST loop runs in **`1.007s`**.

We also caught a subtle async test bug under high parallel CPU load: a `tokio::time::sleep(Duration::from_millis(50))` in an SSE test woke up *before* the spawned handler subscribed, causing `stream.read()` to hang indefinitely. Replacing fixed sleeps with `tokio::sync::Notify` + `tokio::time::timeout` made the test deterministic and dropped its runtime to **`0.010s`**.

---

## 5. Why `cargo check` Feels Slow (and How to Fix It)

`cargo check` (`rustc --emit=metadata`) skips LLVM codegen and linking, yet in large workspaces it can still take 15–66 seconds when switching commands. The culprit is **feature-unification hash churn**:

- `cargo check -p djangors-orm --lib` resolves dependencies with **Hash A**.
- `cargo check --workspace --lib` resolves them with **Hash B**.
- `cargo test` resolves them with **Hash C** (`dev-dependencies` included).
- `rust-analyzer` running `--workspace --all-targets` resolves **Hash D** and contends on `target/debug/.cargo-lock`.

Four fixes eliminate the churn:

1. **Scope `rust-analyzer` to the active crate** (`.zed/settings.json` / VS Code): set `check.workspace = false`, `check.allTargets = false`, `cargo.targetDir = true`, and `cachePriming.enable = false`.
2. **Use `sccache` in `~/.cargo/config.toml`**: caches compiled crates across feature hashes, branches, and `cargo clean` (`SCCACHE_CACHE_SIZE = "4G"`).
3. **Run `bacon check -p <crate>`** in a side terminal for continuous background diagnostics.
4. **Know when NOT to add `RUSTFLAGS`**: `rustc` 1.90+ on Linux `x86_64` already links with `rust-lld` by default. Adding custom `RUSTFLAGS` for `lld` or `mold` invalidates caches whenever an editor or tool invokes Cargo without matching flags.

---

## 6. Faster, Smaller CI (`CARGO_INCREMENTAL=0`)

`target/debug/incremental/` routinely grows to 2.4+ GiB, bloats CI cache restore/save steps, and forces extra LLVM codegen units. Setting `CARGO_INCREMENTAL: "0"` in CI (`.circleci/config.yml`) shrinks caches and speeds up clean CI builds.

---

## 7. Hard-Capping `target/` Disk Size Automatically

On stable Rust, `target/` is append-only (even nightly `cargo -Z gc` only cleans `~/.cargo` global caches, not per-project `target/`).

Djangors ships [`scripts/trim-target.sh`](https://github.com/QUANTORA-ltd/djangors/blob/main/scripts/trim-target.sh) to enforce a hard disk cap (default `4096 MiB`) safely via `flock -n` build-safety guards and staged pruning (ephemeral files → nested test targets → `incremental/` → keeping the 2 newest hashes per crate in `deps/` → LRU eviction by `mtime` while preserving top-level release binaries like `target/release/dj`). On our workspace, it immediately trimmed `target/` from **`9248 MiB` down to `3641 MiB`**.
