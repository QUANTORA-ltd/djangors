//! Optional RED metrics middleware and a bearer-token-protected Prometheus endpoint.
//!
//! Enable the `metrics` feature to use this module. The first call to
//! [`metrics_layer`] or [`metrics_handler`] installs the process-wide metrics
//! recorder. Install it before installing another global recorder. Application
//! metrics emitted through the `metrics` crate share the same Prometheus registry.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::Instant;

use hyper::http::{header, Request as HttpRequest, Response as HttpResponse, StatusCode};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tower::{Layer, Service};

use crate::{Handler, PathParams, Request, Response};

static PROMETHEUS: OnceLock<PrometheusHandle> = OnceLock::new();

fn handle() -> &'static PrometheusHandle {
    PROMETHEUS.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("djangors metrics recorder could not be installed; install it before another global recorder")
    })
}

/// Tower layer that records HTTP request totals, durations, and in-flight requests.
#[derive(Clone, Debug, Default)]
pub struct MetricsLayer;

/// Create the metrics middleware layer, installing the Prometheus recorder on first use.
///
/// Wrap the application service with this layer. Route labels use the router's matched
/// pattern when present and `unmatched` otherwise. In-flight requests are counted globally
/// because their route is not known before the router runs.
pub fn metrics_layer() -> MetricsLayer {
    let _ = handle();
    MetricsLayer
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { inner }
    }
}

/// Service wrapper created by [`MetricsLayer`].
#[derive(Clone, Debug)]
pub struct MetricsService<S> {
    inner: S,
}

struct InFlight;

impl Drop for InFlight {
    fn drop(&mut self) {
        metrics::gauge!("http_requests_in_flight").decrement(1.0);
    }
}

impl<S, B, ResBody> Service<HttpRequest<B>> for MetricsService<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse<ResBody>> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = HttpResponse<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: HttpRequest<B>) -> Self::Future {
        let method = request.method().as_str().to_owned();
        let request_route = request
            .extensions()
            .get::<crate::MatchedRoute>()
            .map(|route| route.as_str().to_owned());
        metrics::gauge!("http_requests_in_flight").increment(1.0);
        let _in_flight = InFlight;
        let started = Instant::now();
        let future = self.inner.call(request);
        Box::pin(async move {
            let result = future.await;
            let (route, status) = match &result {
                Ok(response) => (
                    response
                        .extensions()
                        .get::<crate::MatchedRoute>()
                        .map(|route| route.as_str().to_owned())
                        .or(request_route)
                        .unwrap_or_else(|| "unmatched".to_owned()),
                    response.status().as_u16().to_string(),
                ),
                Err(_) => (
                    request_route.unwrap_or_else(|| "unmatched".to_owned()),
                    "error".to_owned(),
                ),
            };
            let elapsed = started.elapsed().as_secs_f64();
            metrics::counter!("http_requests_total", "method" => method.clone(), "route" => route.clone(), "status" => status.clone()).increment(1);
            metrics::histogram!("http_request_duration_seconds", "method" => method, "route" => route, "status" => status).record(elapsed);
            result
        })
    }
}

/// Create a handler that renders Prometheus exposition text when given a valid bearer token.
///
/// The token is compared in constant time. Unauthorized requests receive `401` and do not
/// receive registry contents. The token should be a high entropy secret.
pub fn metrics_handler(token: impl Into<String>) -> impl Handler {
    let token = token.into();
    let _ = handle();
    move |request: Request, _params: PathParams| {
        let token = token.clone();
        async move {
            let valid = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(|provided| {
                    crate::middleware::constant_time_eq(provided.as_bytes(), token.as_bytes())
                })
                .unwrap_or(false);
            if !valid {
                return Ok(Response::text(StatusCode::UNAUTHORIZED, "Unauthorized"));
            }
            let body = handle().render();
            Ok(Response::bytes(
                StatusCode::OK,
                "text/plain; version=0.0.4; charset=utf-8",
                body.into_bytes(),
            ))
        }
    }
}

/// Render the current metrics registry as Prometheus exposition text.
///
/// This is mainly useful when an application needs a custom endpoint. Protect the
/// resulting output with authentication before exposing it.
pub fn render_metrics() -> String {
    handle().render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::http::HeaderValue;
    use tower::ServiceExt;

    #[tokio::test]
    async fn layer_records_red_metrics_and_handler_requires_bearer_token() {
        let service = metrics_layer().layer(tower::service_fn(
            |request: HttpRequest<Bytes>| async move {
                let mut response = HttpResponse::new(Bytes::new());
                response
                    .extensions_mut()
                    .insert(crate::MatchedRoute(std::sync::Arc::from("/health")));
                *response.status_mut() = StatusCode::NO_CONTENT;
                let _ = request;
                Ok::<_, std::convert::Infallible>(response)
            },
        ));
        let _ = service
            .oneshot(HttpRequest::new(Bytes::new()))
            .await
            .unwrap();
        let handler = metrics_handler("secret");
        let build_request = |headers| {
            Request::new(
                hyper::http::Method::GET,
                hyper::http::Uri::from_static("/metrics"),
                headers,
                Bytes::new(),
            )
        };
        let unauthorized = handler
            .call(build_request(Default::default()), PathParams::new())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let mut headers = hyper::http::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        let authorized = handler
            .call(build_request(headers), PathParams::new())
            .await
            .unwrap();
        let body = String::from_utf8_lossy(authorized.body());
        assert!(body.contains("http_requests_total"));
        assert!(body.contains("http_request_duration_seconds"));
        assert!(body.contains("http_requests_in_flight"));
    }
}
