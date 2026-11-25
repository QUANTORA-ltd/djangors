# Shipping Go-Sized Web Binaries in Rust: How a Batteries-Included Framework Hits 4 MiB

One of the most common objections to Rust web frameworks is binary bloat: "Why use Rust if a web app compiles to a 60 MiB binary and a 15 GiB `target/` directory? I'll just use Go."

The truth is that Cargo's defaults are built for maximum debugging ergonomics, not deployment size. With the right profile and feature architecture, a full batteries-included Rust web framework compiles to **Go-sized static binaries**.

## Measured Binary Sizes in Djangors 1.0

Our CircleCI pipeline enforces hard byte ceilings on two real example apps built with `cargo build --release`:

| Application | Scope | Stripped Release Size |
|---|---|---|
| `examples/minimal-facade` | Postgres-only web app (`default-features = false, features = ["postgres"]`) | **4.00 MiB** |
| `examples/polls` | Full stack (ORM + Admin + Auth + Sessions + Templates) | **11.51 MiB** |

---

## 1. The Release Profile (`dj new` Ships This by Default)

Every project scaffolded with `dj new` includes:

```toml
[profile.release]
opt-level = "z"
lto = "thin"
strip = true
codegen-units = 1
panic = "abort"
```

- **`opt-level = "z"`** optimizes for binary size (often shrinking code by 20–35% over `opt-level = 3` with minimal HTTP throughput difference).
- **`lto = "thin"`** performs cross-crate dead-code elimination across all 38 workspace crates and third-party dependencies.
- **`strip = true`** strips DWARF debug info and symbol tables directly during linking.
- **`codegen-units = 1`** gives LLVM maximum visibility to inline and deduplicate monomorphized generics.
- **`panic = "abort"`** removes stack-unwinding tables from the binary.

---

## 2. Feature-Gating & Backend Isolation

The `djangors` facade crate enables strict feature-gating so you never pay for unused batteries or database drivers:

```toml
[dependencies]
djangors = { version = "1.0", default-features = false, features = ["postgres", "rest", "auth"] }
```

In CI, we enforce backend isolation with `cargo tree`:
- A `postgres`-only build **never** pulls in `libsqlite3-sys` (C SQLite).
- A `sqlite`-only build **never** pulls in `sqlx-postgres`.

That guarantees a minimal Postgres app only links the exact HTTP, ORM, and Postgres driver code it uses—landing cleanly at **4.00 MiB**.

---

## 3. Multi-Stage Docker Images Under 25 MiB

Pair the stripped release binary with a `scratch` or `distroless/cc-debian12` final stage:

```dockerfile
FROM rust:1.96 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin my_app

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/target/release/my_app /app/server
ENTRYPOINT ["/app/server"]
```

No Python interpreter, no `node_modules`, no virtualenv—just a single 4–12 MiB binary that cold-starts in milliseconds.
