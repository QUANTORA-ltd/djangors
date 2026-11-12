# How to Use htmx

## Problem

You use [htmx](https://htmx.org) to update parts of a page, and want one view to return
either a small HTML fragment (for htmx) or the full page (for a normal visit), plus a way
to send htmx instructions such as "redirect" or "trigger this event" from the server.

## Solution

`Request` can tell you whether htmx sent the request, and `Response` has builders for
htmx's response headers. This is the Djangors equivalent of `django-htmx`.

```rust,compile
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

async fn search(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let results = "<ul><li>First result</li></ul>";
    let response = if req.is_htmx() {
        // htmx swaps this fragment into the element named by `hx-target`.
        Response::html(StatusCode::OK, results)
    } else {
        Response::html(
            StatusCode::OK,
            format!("<html><body><h1>Search</h1>{results}</body></html>"),
        )
    };
    // The same URL returns two different bodies, so tell caches to keep them apart.
    Ok(response.header("Vary", "HX-Request"))
}

async fn save(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    // ... save the form ...
    Ok(Response::new(StatusCode::NO_CONTENT)
        .hx_trigger("saved")       // fire a `saved` event in the browser
        .hx_push_url("/items/42")) // update the address bar
}
```

**Always add `Vary: HX-Request`** when a URL answers htmx and normal requests
differently. Without it, a cache (including Djangors' own
[`CacheLayer`](cache-views.md)) or the browser's back button can show the bare fragment
as if it were the whole page.

## Request helpers

| Method | Header | Returns |
| :--- | :--- | :--- |
| `req.is_htmx()` | `HX-Request` | `true` for requests sent by htmx |
| `req.is_htmx_boosted()` | `HX-Boosted` | `true` for links and forms using `hx-boost` |
| `req.htmx_target()` | `HX-Target` | `id` of the element being updated |
| `req.htmx_trigger()` | `HX-Trigger` | `id` of the element that triggered the request |
| `req.htmx_current_url()` | `HX-Current-URL` | The page URL in the browser |

## Response builders

| Method | Header | Effect in the browser |
| :--- | :--- | :--- |
| `.hx_trigger("event")` | `HX-Trigger` | Fires a client-side event |
| `.hx_redirect("/url")` | `HX-Redirect` | Full-page redirect |
| `.hx_refresh()` | `HX-Refresh: true` | Reloads the page |
| `.hx_push_url("/url")` | `HX-Push-Url` | Adds the URL to browser history |
| `.hx_retarget("#css")` | `HX-Retarget` | Swaps into a different element |
| `.hx_reswap("outerHTML")` | `HX-Reswap` | Changes how the content is swapped in |

A value that isn't allowed in an HTTP header, such as one containing a newline, is
skipped with a warning in the logs instead of crashing the request.
