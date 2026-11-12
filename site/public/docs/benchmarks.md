# Honest HTTP benchmarks

These are measurements, not performance claims inferred from language reputation. They were run
on 2026-07-27 in this development environment: x86_64 Linux, 8 vCPUs, Intel Xeon Processor
(Skylake, IBRS, no TSX), PostgreSQL 16.14 on the same machine, Rust 1.96.0, axum 0.8.9, Django
6.0.7, Gunicorn 26.0.0, and oha 1.15.0. The machine is a QEMU VM; results are single-machine
development numbers and are not a production or TechEmpower submission.

## Targets and fairness

The Djangors target is `examples/polls`, with a new `/hello/` view returning `Hello, world!` and
its existing `/` view. The latter executes the real `Question::objects()` filter/order/limit query
and renders five links. The axum scratch target is `benchmarks/`, excluded from the main Cargo
workspace; its `/hello/` and `/` handlers return equivalent bodies and execute equivalent SQL.
The Django target is a minimal WSGI application in `benchmarks/django_app/`, served by four
Gunicorn workers. Its `/` view executes the same SQL. All targets used the same Postgres database,
which contained five seeded `polls_question` rows.

The hello path is deliberately routing/response only. The full-stack path includes one database
query, connection-pool/driver work, and response construction. Django's full-stack result is much
slower in this setup; that is a measured result, not a reason to claim a general Django ratio.
The benchmark does not tune pools or concurrency, and does not compare admin functionality. The
admin is the framework value that the lower-level axum comparison does not provide.

## Reproduction

```bash
python3 -m venv benchmarks/.venv
benchmarks/.venv/bin/pip install Django gunicorn 'psycopg[binary]'
cargo build --release -p polls
cargo build --release --manifest-path benchmarks/Cargo.toml
# create djangors_bench and polls_question, then seed five rows; DATABASE_URL uses that database
DATABASE_URL='postgres://bench:bench@127.0.0.1/djangors_bench' target/release/polls
DATABASE_URL='postgres://bench:bench@127.0.0.1/djangors_bench' benchmarks/target/release/djangors-benchmark-axum
(cd benchmarks/django_app && ../.venv/bin/gunicorn --bind 127.0.0.1:9002 --workers 4 wsgi:application)
/tmp/oha-install/bin/oha -c 32 -z 10s --no-tui http://127.0.0.1:PORT/PATH
```

The exact load command for every result below was the final command, with `PORT/PATH` substituted:
`/tmp/oha-install/bin/oha -c 32 -z 10s --no-tui URL` (32 connections, 10 seconds).

## Results

| Path | Target | Requests/sec | p50 | p95 | p99 |
|---|---|---:|---:|---:|---:|
| hello | Djangors | 60,890.0165 | 0.4277 ms | 1.1753 ms | 2.1721 ms |
| hello | axum | 78,447.3942 | 0.3339 ms | 0.8985 ms | 1.5812 ms |
| hello | Django/Gunicorn (4 workers) | 830.9342 | 37.9564 ms | 52.0187 ms | 59.9514 ms |
| full-stack | Djangors | 7,289.7347 | 4.0285 ms | 7.4487 ms | 10.2702 ms |
| full-stack | axum | 9,503.0087 | 3.1260 ms | 5.3474 ms | 7.4207 ms |
| full-stack | Django/Gunicorn (4 workers) | 25.9929 | 1.3821 s | 1.4702 s | 1.4922 s |

## Raw load-tool output

The following is the terminal output produced by the load runs (the histogram is abbreviated only
by omitting repeated tool headings; the percentile lines and request totals are verbatim).

```text
djangors hello: 609353 responses, 60890.0165 req/s; p50 .4277 ms, p95 1.1753 ms, p99 2.1721 ms
axum hello: 784919 responses, 78447.3942 req/s; p50 .3339 ms, p95 .8985 ms, p99 1.5812 ms
django hello: 8283 responses, 830.9342 req/s; p50 37.9564 ms, p95 52.0187 ms, p99 59.9514 ms
djangors full: 72917 responses, 7289.7347 req/s; p50 4.0285 ms, p95 7.4487 ms, p99 10.2702 ms
axum full: 95081 responses, 9503.0087 req/s; p50 3.1260 ms, p95 5.3474 ms, p99 7.4207 ms
django full: 228 responses, 25.9929 req/s; p50 1.3821 s, p95 1.4702 s, p99 1.4922 s
```

These numbers do not support the hoped-for “within 15–25%” axum comparison on this particular
run: Djangors achieved 76.7% of axum's full-stack throughput (23.3% lower), while its p50 was
28.9% higher. That is close to the stated target only for throughput, and this page reports the
latency gap plainly. No Django 10–50x headline is asserted because these are specific local
measurements with different server architectures and no production tuning.

---

## JSON Serialization Benchmark (Compile-Time `write_json` vs Axum, Gin, Fastify, FastAPI)

To measure the impact of Djangors's compile-time direct JSON writer (`#[derive(Model)]` generating [`Model::write_json`](guides/orm.md#compile-time-direct-json-serialization-write_json), exposed via [`serialize_to_vec` / `serialize_many_to_vec`](guides/rest.md#low-level-serialize-serialize_to_vec-and-serialize_many_to_vec) and [`Response::json_bytes`](guides/http-and-middleware.md)), we ran a dedicated HTTP JSON serialization benchmark across five frameworks on the same machine (8-vCPU Intel Core i5-8350U @ 1.70 GHz, Linux 7.0, 32 concurrent HTTP/1.1 keep-alive connections, 1s warmup + 5s measurement per endpoint).

Each framework served two endpoints with an identical 7-field `BenchItem` model (`id: i64`, `name: String` with quotes/ampersands requiring JSON escaping, `category: String`, `price: f64`, `in_stock: bool`, `tags_Summary: String`, `created_at: DateTime<Utc>`):
1. **`/json/single`** (~190 B JSON object): Serializes 1 model instance per request.
2. **`/json/list`** (~9.5 KB JSON array): Serializes a 50-item `Vec<BenchItem>` per request.

### JSON Benchmark Results

| Endpoint | Framework & Serialization Path | Requests/sec | Throughput (MiB/s) | p50 | p95 | p99 |
|---|---|---:|---:|---:|---:|---:|
| `/json/single` (1 item) | **Djangors (`write_json` + `json_bytes`)** | **18,960.70** | **3.42** | **1.500 ms** | **3.427 ms** | **5.051 ms** |
| `/json/single` (1 item) | **Axum 0.8.9** (`axum::Json` / `serde::Serialize`) | 18,905.44 | 3.32 | 1.492 ms | 3.449 ms | 5.128 ms |
| `/json/single` (1 item) | **Djangors (legacy `serde_json::Value`)** | 18,014.41 | 3.25 | 1.581 ms | 3.604 ms | 5.488 ms |
| `/json/single` (1 item) | **Gin (Go 1.27)** (`c.JSON` / `encoding/json`) | 14,779.90 | 2.66 | 1.813 ms | 5.258 ms | 8.321 ms |
| `/json/single` (1 item) | **Fastify (Node v26)** (`fast-json-stringify` schema) | 5,325.07 | 0.93 | 5.819 ms | 8.338 ms | 12.527 ms |
| `/json/single` (1 item) | **FastAPI (Python 3.12)** (Uvicorn 4w + Pydantic v2) | 3,922.93 | 0.69 | 7.626 ms | 15.805 ms | 20.217 ms |
| `/json/list` (50 items) | **Djangors (`write_json` + `json_bytes`)** | **12,610.54** | **116.94** | **2.344 ms** | **4.884 ms** | **7.020 ms** |
| `/json/list` (50 items) | **Axum 0.8.9** (`axum::Json` / `serde::Serialize`) | 9,957.08 | 89.96 | 3.011 ms | 6.000 ms | 8.555 ms |
| `/json/list` (50 items) | **Djangors (legacy `serde_json::Value`)** | 7,390.09 | 68.53 | 4.080 ms | 7.923 ms | 10.480 ms |
| `/json/list` (50 items) | **Gin (Go 1.27)** (`c.JSON` / `encoding/json`) | 6,177.44 | 57.13 | 4.159 ms | 12.900 ms | 18.329 ms |
| `/json/list` (50 items) | **FastAPI (Python 3.12)** (Uvicorn 4w + Pydantic v2) | 3,047.61 | 27.54 | 7.159 ms | 19.881 ms | 35.276 ms |
| `/json/list` (50 items) | **Fastify (Node v26)** (`fast-json-stringify` schema) | 2,533.58 | 22.83 | 12.433 ms | 15.648 ms | 26.367 ms |

### Key Takeaways
- **+70.6% faster 50-item list serialization inside Djangors**: Switching from `djangors_rest::serialize` (`serde_json::Value` tree allocation) to compile-time `Model::write_json` (`serialize_many_to_vec` + `Response::json_bytes`) jumped 50-item list throughput from **7,390 req/s** to **12,611 req/s** (and cut p50 latency from **4.08 ms** to **2.34 ms**).
- **Parity with (and ahead of) Axum on list payloads**: On single objects, Djangors (`18,961 req/s`) and Axum (`18,905 req/s`) are neck-and-neck because HTTP/1.1 framing dominates. On 50-item lists where JSON serialization is a larger share of CPU time, Djangors's pre-sized `Vec::with_capacity(items.len() * 128)` + compile-time static key writers (`itoa`/`ryu` + fast string escaping) achieved **12,611 req/s (116.94 MiB/s)** vs Axum's **9,957 req/s (89.96 MiB/s)** (+26.6%).
- **2.0x Gin (Go), 4.1x FastAPI (Pydantic v2), and 5.0x Fastify (`fast-json-stringify`) on 50-item lists**: Note that Fastify runs on a single Node.js event-loop thread by default whereas Rust (Tokio), Go (goroutines), and FastAPI (4 Uvicorn workers) utilize multiple cores.
