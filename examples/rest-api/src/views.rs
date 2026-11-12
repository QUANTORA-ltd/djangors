use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use djangors::core::extract::{FromRequest, Json};
use djangors::core::resilience::{retry, with_timeout, BreakerError, CircuitBreaker, RetryPolicy};
use djangors::core::versioning::ApiVersion;
use djangors::core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors::db::{BindValue, Database, DbError};
use djangors::orm::Model;
use djangors::rest::{ApiKeyAuth, OpenApiBuilder};
use djangors::tasks::enqueue_in;

use crate::models::Order;

#[derive(serde::Deserialize)]
pub struct CreateOrderRequest {
    pub reference: String,
    pub amount_cents: i64,
}

#[derive(serde::Serialize)]
struct ReceiptTaskPayload<'a> {
    order_id: i64,
    reference: &'a str,
    amount_cents: i64,
}

/// Serves the OpenAPI 3.1 specification with `ApiKeyAuth` security scheme.
pub async fn openapi_spec(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let mut builder = OpenApiBuilder::new("Orders API", "1.0.0");
    builder
        .version("2.0.0")
        .api_key_security()
        .register::<Order>("/v2/orders");
    Response::json(StatusCode::OK, &builder.build())
}

/// Lists active (non-soft-deleted) orders.
///
/// Works on both `/v1/orders/` (deprecated) and `/v2/orders/`, echoing the
/// resolved [`ApiVersion`] in the JSON envelope.
pub async fn list_orders(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let ApiVersion(version) = ApiVersion::from_request(&req).await?;
    let ApiKeyAuth(key) = ApiKeyAuth::from_request(&req).await?;
    let db = req.require_state::<Database>()?;

    let orders = Order::objects()
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    Response::json(
        StatusCode::OK,
        &serde_json::json!({
            "api_version": version,
            "caller": key.name,
            "orders": orders,
        }),
    )
}

/// Creates an order under a distributed advisory lock and enqueues a receipt
/// task in the **same** database transaction (transactional outbox).
///
/// Requires an API key with the `orders:write` scope.
pub async fn create_order(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let ApiVersion(_version) = ApiVersion::from_request(&req).await?;
    let ApiKeyAuth(key) = ApiKeyAuth::from_request(&req).await?;
    if !key.has_scope("orders:write") {
        return Err(DjangorsError::Forbidden(
            "API key lacks 'orders:write' scope".to_string(),
        ));
    }

    let Json(body) = Json::<CreateOrderRequest>::from_request(&req).await?;
    let db = req.require_state::<Database>()?;

    let lock_key = format!("order:create:{}", body.reference);
    let reference = body.reference.clone();
    let amount_cents = body.amount_cents;

    let outcome = djangors::db::locks::with_lock(db, &lock_key, move |conn| {
        Box::pin(async move {
            let dialect = conn.dialect();
            let p1 = dialect.placeholder(1);
            let p2 = dialect.placeholder(2);
            let p3 = dialect.placeholder(3);
            let p4 = dialect.placeholder(4);
            let sql = format!(
                "INSERT INTO orders_order (reference, amount_cents, status, created_at) \
                 VALUES ({p1}, {p2}, {p3}, {p4}) RETURNING id"
            );
            let now = chrono::Utc::now();
            let row = conn
                .fetch_one(
                    &sql,
                    &[
                        BindValue::Text(reference.clone()),
                        BindValue::I64(amount_cents),
                        BindValue::Text("pending".to_string()),
                        BindValue::DateTime(now),
                    ],
                )
                .await?;
            let order_id = row
                .try_i64(0)?
                .ok_or_else(|| DbError::TransactionFailed("missing order id".into()))?;

            let task_id = enqueue_in(
                conn,
                "orders.send_receipt",
                &ReceiptTaskPayload {
                    order_id,
                    reference: &reference,
                    amount_cents,
                },
            )
            .await
            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;

            Ok::<(i64, i64), DbError>((order_id, task_id))
        })
    })
    .await
    .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    match outcome {
        Some((order_id, task_id)) => Response::json(
            StatusCode::CREATED,
            &serde_json::json!({
                "order_id": order_id,
                "outbox_task_id": task_id,
            }),
        ),
        None => Err(DjangorsError::api(
            StatusCode::CONFLICT,
            "lock_unavailable",
            "order creation already in progress",
        )),
    }
}

/// Demonstrates `RetryPolicy`, `with_timeout`, and `CircuitBreaker` for an
/// upstream shipping-quote call.
pub async fn upstream_quote(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let ApiKeyAuth(_key) = ApiKeyAuth::from_request(&req).await?;
    let breaker = req.require_state::<CircuitBreaker>()?;

    let policy = RetryPolicy::new()
        .max_attempts(3)
        .initial_delay(Duration::from_millis(5))
        .max_delay(Duration::from_millis(20))
        .jitter(false);

    let attempts = AtomicUsize::new(0);
    let result = breaker
        .call(retry(
            &policy,
            || {
                let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    with_timeout(Duration::from_millis(200), async move {
                        if n < 2 {
                            Err("transient upstream 503")
                        } else {
                            Ok(499_i64)
                        }
                    })
                    .await
                    .map_err(|_| "upstream timed out")?
                }
            },
            |_| true,
        ))
        .await;

    match result {
        Ok(shipping_cents) => Response::json(
            StatusCode::OK,
            &serde_json::json!({
                "shipping_cents": shipping_cents,
                "attempts": attempts.load(Ordering::SeqCst),
            }),
        ),
        Err(BreakerError::Open) => Err(DjangorsError::api(
            StatusCode::SERVICE_UNAVAILABLE,
            "circuit_open",
            "upstream quote service circuit breaker is open",
        )),
        Err(BreakerError::Inner(msg)) => Err(DjangorsError::Internal(msg.to_string())),
    }
}
