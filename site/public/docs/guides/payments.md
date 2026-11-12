# Payments (`djangors-contrib-payments`)

`djangors-contrib-payments` is a provider-agnostic payment integration crate, shipping a
Paystack implementation first. It's built around two hard requirements for anything touching
real money: **amounts are never a float**, and **a webhook redelivery or duplicate initiate call
can never double-process a charge**.

## Amounts: integer minor units, never a float

Every amount in this crate is an `i64` in the currency's minor unit (kobo for NGN, cents for
USD), never `f64` and never `rust_decimal::Decimal`. This is the same convention Paystack's and
Stripe's own APIs use on the wire, so there's no float-precision problem to solve in the first
place: `5000` means ₦50.00, not `50.0`.

## The `PaymentProvider` trait

```rust,compile
use djangors_contrib_payments::{
    InitiateChargeRequest, InitiateChargeResponse, PaymentError, PaymentProvider, RefundResult,
};

async fn charge_example(provider: &dyn PaymentProvider) -> Result<InitiateChargeResponse, PaymentError> {
    let req = InitiateChargeRequest {
        email: "customer@example.com".to_string(),
        amount_minor: 50_000, // NGN 500.00
        currency: "NGN".to_string(),
        reference: "order-1234".to_string(),
        callback_url: Some("https://example.com/callback".to_string()),
    };
    provider.initiate(&req).await
}
```

Every provider implements four operations: `initiate` (start a charge, get a redirect URL back),
`verify` (confirm a transaction's real status directly with the provider; never trust a
client-side "success" redirect alone), `verify_webhook_signature`, and `refund`.

## `PaystackProvider`

```rust,compile
use djangors_contrib_payments::PaystackProvider;

fn make_provider(secret_key: &str) -> PaystackProvider {
    PaystackProvider::new(secret_key.to_string())
}
```

`PaystackProvider::new(secret_key)` talks to the real Paystack API
(`https://api.paystack.co`). Get your secret key from the Paystack dashboard and load it via
`#[derive(Settings)]` (see the [Settings guide](settings.md)) rather than hardcoding it.

For sandbox/testing against a mock server, or to route through a proxy in front of Paystack,
use `with_base_url(secret_key, base_url)` — it points every request at `base_url` instead of the
real `https://api.paystack.co`:

```rust,compile
# fn main() {
use djangors_contrib_payments::PaystackProvider;

let live = PaystackProvider::new("sk_live_..."); // real https://api.paystack.co
let sandbox = PaystackProvider::with_base_url("sk_test_...", "http://localhost:8080"); // mock server
let proxied = PaystackProvider::with_base_url("sk_live_...", "https://paystack-proxy.internal");
# let _ = (live, sandbox, proxied);
# }
```

## `BachsProvider`

[Bachs](https://bachs.io) is a payments/billing platform aimed at internet businesses with
African settlement. `BachsProvider` implements the same `PaymentProvider` trait as Paystack, so
it is a drop-in alternative for one-time charges.

```rust,compile
use djangors_contrib_payments::BachsProvider;

fn make_provider(secret_key: &str) -> BachsProvider {
    BachsProvider::new(secret_key.to_string())
}
```

There are three constructors. `new(secret_key)` talks to the production API
(`https://api.bachs.io`), `sandbox(secret_key)` points at `https://sandbox-api.bachs.io`, and
`with_base_url(secret_key, base_url)` routes everything at an arbitrary base URL (useful for a
mock server or a proxy). Build against sandbox first — going live is a key swap (`sk_sandbox_`
→ `sk_live_`), not a code change:

```rust,compile
# fn main() {
use djangors_contrib_payments::BachsProvider;

let live = BachsProvider::new("sk_live_..."); // real https://api.bachs.io
let sandbox = BachsProvider::sandbox("sk_sandbox_..."); // https://sandbox-api.bachs.io
# let _ = (live, sandbox);
# }
```

### Amounts: decimal strings on the wire

This is the part most likely to trip someone up. The `PaymentProvider` trait speaks in
`amount_minor: i64` (kobo, cents), and you keep using minor units throughout — but Bachs's own
API uses decimal strings at the currency's precision and explicitly never minor units. The
provider converts in both directions using exact integer arithmetic, driven by a
currency-exponent table: 0 decimals for currencies like JPY and XOF, 3 for currencies like KWD
and BHD, and 2 for everything else (including NGN, USD, GHS, KES, ZAR). You never see a decimal
string; the conversion happens at the boundary. So `amount_minor: 123456` with `"NGN"` becomes
`"1234.56"` on the wire, and a `"1234.56"` response round-trips back to `123456`.

### `initiate` / `verify` / `refund`

The one deviation from Paystack you must not miss: **Bachs has no lookup-by-merchant-reference
endpoint**, so `verify(reference)` and `refund(reference, ..)` interpret their argument as the
Bachs **checkout id**, not your own merchant reference. `initiate` returns that checkout id in
the `access_code` field of `InitiateChargeResponse` (Bachs has no "access code" concept of its
own) and the redirect URL in `authorization_url`. **The caller must persist `access_code` after
`initiate`**, or it cannot verify or refund the charge later. A reader who misses this ends up
with a broken integration.

### Webhooks

Bachs signs deliveries with two headers, `X-Bachs-Timestamp` and `X-Bachs-Signature`, where the
signature is an HMAC-SHA256 hex digest of `"{timestamp}.{raw_body}"`. Because the single-argument
`verify_webhook_signature` has nowhere to put the timestamp, the trait gained
`verify_webhook_signature_with_timestamp(raw_body, timestamp, signature)`, whose default
implementation ignores the timestamp so Paystack is unaffected. Note that `BachsProvider`'s plain
`verify_webhook_signature` deliberately returns `false` unconditionally — it fails closed rather
than pretending to verify — so Bachs callers **must** use the timestamped method.

`handle_bachs_webhook` verifies, parses, and records the delivery in one call. Pass the raw body
bytes exactly as received, never a re-serialized copy, or the signature will not match. The
`collection.succeeded` event is the one treated as payment confirmation.

### Paystack or Bachs?

At a factual level: Paystack verifies and refunds by your merchant reference and signs webhooks
with a single HMAC-SHA512 header; Bachs verifies and refunds by checkout id, uses multi-currency
decimal amounts, and signs webhooks with a timestamped HMAC-SHA256 pair. Pick the provider whose
API and settlement model match your market.

## Idempotent transaction recording

`Transaction`'s `reference` column has a real **database-level UNIQUE constraint**, not an
application-level check-then-insert, which would race under a concurrent webhook redelivery or a
double-clicked "pay now" button. `record_charge_initiated` relies on that constraint directly:

```rust,illustrative
use djangors_contrib_payments::{record_charge_initiated, Transaction};
use djangors_orm::djangors_db::Database;

async fn start_checkout(db: &Database, reference: &str) -> Result<Transaction, djangors_contrib_payments::PaymentError> {
    // Called twice with the same `reference` (a retry, a duplicate webhook) returns the SAME
    // row both times - it never creates a second one and never errors.
    record_charge_initiated(db, reference, "paystack", 50_000, "NGN", None).await
}
```

## Transaction statuses

`Transaction.status` is a plain string column, but the crate models it as the
`TransactionStatus` enum so you never hand-type a lowercase string. The variants are
`Pending`, `Success`, `Failed`, and `Refunded`, stored lowercased, and the enum round-trips
through `as_str()`, `Display`, and `FromStr`:

```rust,compile
# use std::str::FromStr;
# fn main() {
use djangors_contrib_payments::TransactionStatus;

assert_eq!(TransactionStatus::Pending.as_str(), "pending");
assert_eq!(TransactionStatus::Success.to_string(), "success");

// DB string -> enum, and back.
let back: TransactionStatus = "refunded".parse().expect("valid status");
assert_eq!(back, TransactionStatus::Refunded);
assert_eq!(TransactionStatus::from_str("failed").unwrap(), TransactionStatus::Failed);
# }
```

`mark_transaction_status` flips an existing row (found by `reference`) to a new status,
optionally replacing its stored metadata. It returns `Ok(None)` when no row exists for that
`reference` — a sign the initiate step was skipped:

```rust,illustrative
use djangors_contrib_payments::{TransactionStatus, mark_transaction_status};
use djangors_orm::djangors_db::Database;

async fn flag_failed(db: &Database, reference: &str) -> Result<(), djangors_contrib_payments::PaymentError> {
    // Optionally replace the row's metadata with a reason.
    match mark_transaction_status(
        db,
        reference,
        TransactionStatus::Failed,
        Some(&serde_json::json!({"reason": "card_declined"})),
    )
    .await?
    {
        Some(txn) => { /* txn.status == "failed" */ }
        None => { /* no row with that reference - initiate wasn't recorded */ }
    }
    Ok(())
}
```

Fetch a row directly with `find_by_reference(db, reference)`, which returns
`Result<Option<Transaction>, PaymentError>` — `None` when no row matches:

```rust,illustrative
use djangors_contrib_payments::{Transaction, find_by_reference};
use djangors_orm::djangors_db::Database;

async fn lookup(db: &Database, reference: &str) -> Result<Option<Transaction>, djangors_contrib_payments::PaymentError> {
    find_by_reference(db, reference).await
}
```

## Webhook handling

`handle_paystack_webhook` replicates the real, production-proven order of operations for
processing an incoming webhook safely:

1. Verify the `x-paystack-signature` header (HMAC-SHA512, constant-time) against the **raw
   request body bytes**, before ever parsing any JSON. A bad signature is rejected without the
   payload being touched at all.
2. Only then parse the JSON body.
3. Require **both** `event == "charge.success"` **and** `data.status == "success"` before
   treating the charge as confirmed. Checking only one of these would be a real security gap.
4. Idempotently record or update the transaction, so a genuine webhook redelivery is a harmless
   no-op, not a duplicate credit.

```rust,illustrative
use djangors_contrib_payments::{handle_paystack_webhook, PaystackProvider};
use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
use djangors_orm::djangors_db::Database;

pub async fn paystack_webhook_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("no database in request state".into()))?;
    let provider = req
        .state::<PaystackProvider>()
        .ok_or_else(|| DjangorsError::Internal("no PaystackProvider in request state".into()))?;

    let signature = req
        .header("x-paystack-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| DjangorsError::Unauthorized("missing signature header".into()))?;
    let body = req.body_bytes().await;

    handle_paystack_webhook(provider, db, body, signature)
        .await
        .map_err(|e| DjangorsError::Unauthorized(e.to_string()))?;

    Ok(Response::text(StatusCode::OK, "ok"))
}
```

## Verifying and reconciling charges

A charge is not confirmed when the customer's browser says it is — a client-side "success" page
can be forged or simply never arrive. The full lifecycle always ends in a **server-side**
`verify`:

1. `initiate` the charge and keep the returned `InitiateChargeResponse.reference`.
2. `record_charge_initiated` a `Pending` row keyed by that `reference` *before* redirecting, so a
   late webhook or a duplicate initiate finds (and reuses) it.
3. Redirect the customer to `authorization_url`.
4. On the callback/return, call `provider.verify(&reference)` ourselves — never trust the
   redirect alone.
5. `mark_transaction_status` with the verified status.

The signature-verified [webhook](#webhook-handling) is the out-of-band safety net for the same
flow: it both records and confirms in one call, so a payment that never came back through the
redirect is still reconciled.

```rust,illustrative
use djangors_contrib_payments::{
    InitiateChargeRequest, PaymentProvider, PaystackProvider, mark_transaction_status,
    record_charge_initiated,
};
use djangors_orm::djangors_db::Database;

async fn checkout(db: &Database, provider: &PaystackProvider) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Start the charge. Keep the response's reference for verification.
    let initiated = provider
        .initiate(&InitiateChargeRequest {
            email: "customer@example.com".into(),
            amount_minor: 50_000,
            currency: "NGN".into(),
            reference: "order-5678".into(),
            callback_url: Some("https://example.com/callback".into()),
        })
        .await?;

    // 2. Persist a Pending row before redirecting (idempotent on this reference).
    record_charge_initiated(db, &initiated.reference, "paystack", 50_000, "NGN", None).await?;

    // 3. Redirect to initiated.authorization_url ... customer returns via callback.

    // 4. Server-side verification. Never trust the client-side redirect alone.
    let verified = provider.verify(&initiated.reference).await?;

    // 5. Only now persist the confirmed status.
    mark_transaction_status(db, &initiated.reference, verified.status, None).await?;
    Ok(())
}
```

## Not yet included

Stripe/Anchor/Moniepoint providers, HTTP route wiring into any example app (this crate exposes
functions your own handler calls, matching how `djangors-auth` exposes backends rather than
routes; not an oversight), and generic idempotency-key middleware for arbitrary POST APIs (a
separate, broader concern than this crate's own reference-based idempotency) are all deliberately
out of scope for this first slice.
