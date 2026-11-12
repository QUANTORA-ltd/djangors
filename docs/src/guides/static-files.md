# Static Files

Managing CSS, JavaScript, images, and other assets is handled by the `djangors-staticfiles` subsystem. This provides dev-time asset serving, a pluggable storage model, and a production `collectstatic` asset pipeline that generates content-hashed assets and a tracking manifest.

---

## Dev-Time Serving

During local development, Djangors can serve static assets directly from your source directories.

```rust,illustrative
use djangors::staticfiles::StaticFiles;
use std::path::PathBuf;

// Set up source directories
let static_dirs = vec![
    PathBuf::from("src/static"),
    PathBuf::from("static_assets"),
];

let static_files = StaticFiles::new(static_dirs);

// Mount the static files handler in the router
let app = djangors::Router::new()
    .mount("/static/<path:path>", static_files.handler());
```

When a request arrives at `/static/style.css`, the `StaticFiles` handler:
1. Loops through your configured source directories in the order they were defined.
2. Checks if the requested file exists in that directory.
3. Serves the file with the appropriate `Content-Type` header (such as `text/css; charset=utf-8` or `application/javascript; charset=utf-8`).

---

## The `Storage` Trait

Djangors abstracts file read and write operations behind the `Storage` trait, allowing you to use different asset destinations in development versus production:

```rust,illustrative
use async_trait::async_trait;
use djangors::staticfiles::StaticFilesError;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn save(&self, path: &str, content: Vec<u8>) -> Result<String, StaticFilesError>;
    async fn open(&self, path: &str) -> Result<Vec<u8>, StaticFilesError>;
    async fn exists(&self, path: &str) -> Result<bool, StaticFilesError>;
    async fn delete(&self, path: &str) -> Result<(), StaticFilesError>;
    fn url(&self, path: &str) -> String;
}
```

### Supported Implementations
* **`LocalDiskStorage`**: Saves and reads files from a directory on the local filesystem. To protect against directory traversal attacks, it canonicalizes paths and verifies they reside within the root.
* **`S3Storage`**: Interacts with AWS S3 or any S3-compatible service (such as MinIO, Cloudflare R2, or DigitalOcean Spaces) using the `Client` API.

### Configuring `S3Storage`

`S3Storage::new` takes the bucket name, AWS region, an optional custom endpoint (e.g. MinIO), the
access key/secret key pair, and the public base URL used by `url()`:

```rust,compile
# fn main() {
use djangors_staticfiles::{S3Storage, Storage};

let storage: S3Storage = S3Storage::new(
    "my-assets",                          // bucket
    "us-east-1",                          // region
    Some("https://minio.example.com"),    // endpoint; None => https://s3.amazonaws.com
    "AKIAEXAMPLEACCESSKEY",               // access key
    "example-secret-key",                 // secret key
    "https://cdn.example.com",            // public base URL
).unwrap();

let _ = storage.url("css/style.css"); // => "https://cdn.example.com/css/style.css"
# }
```

`S3Storage::new` performs no network I/O — it only builds an S3 client. To create the bucket at
startup, call the async `create_bucket()` (idempotent: already-owned buckets are treated as
success):

```rust,illustrative
let storage = S3Storage::new("my-assets", "us-east-1", None, "key", "secret", "https://cdn.example.com")?;
storage.create_bucket().await?; // BucketAlreadyOwnedByYou / BucketAlreadyExists are OK
```

Both `S3Storage` and `LocalDiskStorage` implement the same [`Storage`](#the-storage-trait) trait, so
`collect_to` and `save_upload` work identically against local disk or an object store.

> [!TIP]
> `S3Storage` is gated behind the default-on `s3` Cargo feature on `djangors-staticfiles` (and `staticfiles-s3` on the `djangors` facade). If you only serve or collect static files to local disk, disable `s3` (`default-features = false`) to omit `rust-s3` and its HTTP/XML dependencies from your build.

---

## Production Collection (`collectstatic`)

In production, serving assets through the Rust application process is inefficient. Instead, you run the collection pipeline to bundle and write your assets to a production storage destination (such as local disk served by Nginx, or an S3 bucket with a CDN).

To collect static files, use the `collect` method (for local disk storage) or `collect_to` (for any `Storage` backend):

```rust,illustrative
// Collects all static files into the "staticfiles/" directory
static_files.collect(Path::new("staticfiles/"))?;
```

You can also trigger this via the command-line utility:
```bash
dj collectstatic
```

### Content Hashing and `manifest.json`
When running the collection process:
1. Djangors crawls all source directories and reads every asset.
2. It computes the SHA-256 hash of each file's contents.
3. It appends the first 8 characters of the hash to the file's filename (e.g. `logo.css` becomes `logo.8bf74a2d.css`), ensuring browsers cache assets forever.
4. It saves the hashed files via the configured storage backend.
5. It generates a `manifest.json` file mapping the original paths to the hashed paths:

```json
{
  "mapping": {
    "css/style.css": "css/style.f8a7e3d1.css",
    "images/logo.png": "images/logo.a8c6e28f.png"
  }
}
```

6. The `manifest.json` is saved in the root of the storage directory, allowing template helpers to resolve path names to their production content-hashed variants.

### The `Manifest` Struct

The manifest written to storage is the Rust struct `djangors_staticfiles::Manifest` — a simple
`{ mapping: HashMap<String, String> }` mapping original relative paths to content-hashed paths.
It implements `serde::{Serialize, Deserialize}`, so it can be (de)serialized directly:

```rust,compile
# fn main() {
use djangors_staticfiles::Manifest;

let manifest = Manifest {
    mapping: std::collections::HashMap::from([
        ("css/style.css".to_string(), "css/style.f8a7e3d1.css".to_string()),
        ("images/logo.png".to_string(), "images/logo.a8c6e28f.png".to_string()),
    ]),
};

let json = serde_json::to_string_pretty(&manifest).unwrap(); // {"mapping": {...}}
let back: Manifest = serde_json::from_str(&json).unwrap();
# let _ = (json, back);
# }
```

`StaticFiles::collect(path)` and `StaticFiles::collect_to(&storage)` both return a `Manifest`.

---

## Uploading User Files (`save_upload`)

`djangors_staticfiles::upload::save_upload` writes an uploaded
file through any `Storage` backend under a collision-avoiding name and returns the stored path:

```rust,illustrative
pub async fn save_upload(
    file: &djangors_core::extract::UploadedFile,
    storage: &dyn Storage,
    dest_dir: &str,
) -> Result<String, StaticFilesError>
```

The stored name sanitizes the client-supplied stem and appends the first 12 hex characters of a
SHA-256 hash of the file contents (`photo.jpg` → `photo_9f2a41c8b3d7.jpg`), so the stored path
never equals the raw client filename and identical content dedupes naturally.

Pair it with the `Multipart` extractor, which parses `multipart/form-data` bodies into
`MultipartData { files: Vec<UploadedFile>, texts: HashMap<String, String> }`:

```rust,illustrative
use djangors_core::extract::{FromRequest, Multipart, UploadedFile};
use djangors_core::{DjangorsError, Request, Response, StatusCode};
use djangors_staticfiles::LocalDiskStorage;
use djangors_staticfiles::upload::save_upload;

async fn upload_handler(req: Request) -> Result<Response, DjangorsError> {
    let Multipart(form) = Multipart::from_request(&req).await?;
    let storage = LocalDiskStorage::new("var/media", "/media/"); // or S3Storage

    let mut saved = Vec::new();
    for file in &form.files {
        let path = save_upload(file, &storage, "avatars").await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        saved.push(path); // e.g. "avatars/avatar_9f2a41c8b3d7.png"
    }
    Ok(Response::text(StatusCode::OK, &format!("stored: {saved:?}")))
}
```

`Storage::url(path)` then turns the stored path into a public URL for the stored file.

---

## Serving Static Files (`StaticFiles::serve`)

`StaticFiles::serve` (`crates/djangors-staticfiles/src/serve.rs`) is the async
handler behind the dev-time serving story:

```rust,illustrative
pub async fn serve(&self, req: Request, params: PathParams) -> Result<Response, DjangorsError>
```

It resolves the `path` path parameter (falling back to stripping a `/static/` prefix), searches the
source directories in order, sets the `Content-Type` from the file extension (`.css`, `.js`,
`.png`, `.jpg`, `.svg`, `.woff`, `.woff2`, `.json`, else `application/octet-stream`), and returns
`404 Not Found` when the file is missing. Use it directly from a custom handler, or use the
pre-built `.handler()` capture for router mounting:

```rust,illustrative
use djangors_staticfiles::StaticFiles;
use std::path::PathBuf;

fn mount_static() -> djangors::Router {
    let static_files = StaticFiles::new(vec![
        PathBuf::from("src/static"),
        PathBuf::from("static_assets"),
    ]);

    djangors::Router::new()
        // handler() captures the config and forwards to serve() on each request:
        .mount("/static/<path:path>", static_files.handler())
}
```

`StaticFiles::handler()` returns a `Clone`-able `StaticFilesHandler` that implements the
`Handler` trait and calls `serve()` internally on every request. If you ever need the resolved
`PathParams` yourself, call `serve(req, params)` directly from a custom `async fn` handler — the
method is public and takes the buffer body `Request` plus `PathParams`, exactly like any other
handler.
