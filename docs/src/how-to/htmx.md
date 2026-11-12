# How to Use htmx

htmx sends ordinary HTTP requests and lets the server return HTML fragments. Djangors
provides request helpers for htmx headers and response builders for htmx response headers.

```rust,illustrative
async fn page(req: djangors_core::Request) -> Result<djangors_core::Response, djangors_core::DjangorsError> {
    if req.is_htmx() {
        Ok(djangors_core::Response::html(djangors_core::StatusCode::OK, "<p>Updated fragment</p>"))
    } else {
        Ok(djangors_core::Response::html(djangors_core::StatusCode::OK, "<html><body><p>Full page</p></body></html>"))
    }
}
```

Use `req.is_htmx_boosted()`, `req.htmx_target()`, `req.htmx_trigger()`, and
`req.htmx_current_url()` to inspect request metadata. Responses can use
`.hx_trigger("saved")`, `.hx_redirect("/account")`, `.hx_refresh()`, `.hx_push_url("/items")`,
`.hx_retarget("#results")`, and `.hx_reswap("outerHTML")`.
