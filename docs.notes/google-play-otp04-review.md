# Google Play OTP-04 Reshape Review

Date: 2026-05-10

Scope: staged Rust and OTP-04 harness changes for the Google Play one-time product pending/approval flow. This is review advice only; no code changes were made as part of the review.

## Findings

### 1. Blocker: raw purchase tokens and user IDs are logged at info level

`src/application/verify_purchase.rs` now logs `external_user_id` and `purchase_token` at `info` level during `verify_purchase`.

Bridge tries to minimize PII and sensitive payment identifiers. Google purchase tokens are long-lived restore/fraud-prevention identifiers, so normal production logs should not contain them.

Advice: remove the log or redact it. If correlation is needed, log app/provider/product type plus a short token hash instead of the raw token.

### 2. Blocker/design risk: OTP success can be downgraded back to pending

The OTP-04 flow can currently regress:

1. `/verify-purchase` sees a slow-card token and stores payment `pending`.
2. Google RTDN/webhook marks the payment `success`.
3. A later verify retry still sees `purchaseState: 2` and writes `pending` again.

The DB payment upsert unconditionally applies the incoming status for the same user, so `success` can be overwritten by `pending`. The shell test avoids this by checking DB before retrying verify, but the application should enforce monotonic payment state itself.

Advice: make one-time payment status monotonic in the source-of-truth write path. At minimum, do not allow terminal/stronger states like `success`, `refunded`, or `cancelled` to regress to `pending`.

### 3. Blocker for real Google flow: pending purchases are acknowledged too early, while webhook completion may not acknowledge

The application layer acknowledges any Google purchase whose acknowledgement state is pending. For one-time products, that can include `purchaseState: 2` pending slow-card purchases.

In mock mode this marks `acknowledged_at` even while the payment is still pending. In production, Google may reject acknowledgement before the purchase is completed; the later one-time-product purchased webhook records success but does not appear to acknowledge the product.

Advice: only acknowledge one-time products once `purchaseState == 0` / Bridge status is non-pending. Also ensure the RTDN completion path acknowledges, or guarantees a post-completion verify happens before Google’s acknowledgement deadline.

### 4. Medium: handler `202 Accepted` decision bypasses product type normalization

The handler returns `202` only for raw `product_type == "one_time"` or `"inapp"`, while the parser accepts normalized aliases such as `"one-time"`, uppercase variants, and trimmed input.

Advice: base the HTTP status decision on the normalized product type, or have the application response expose enough semantic state for the handler to avoid raw-string checks.

### 5. Medium/test reliability: mock fixture header does not appear wired in Rust

The OTP-04 shell script sends `X-Mock-Google-Purchase-Response`, but the Rust mock verifier reads `MOCK_GOOGLE_PURCHASE_RESPONSE` from the process environment. I did not find source code consuming that request header.

Advice: either wire the header intentionally in mock/test-only code or remove it from the harness/docs and use the supported environment-based fixture mechanism.

## Positive notes

- Moving one-time `is_new` detection from subscription existence to payment existence is directionally correct because one-time products should not create subscription rows.
- Moving `get_payment_status_for_provider` into `PaymentReadRepository` compiled cleanly and removes a duplicate webhook-specific method.
- Returning `202 Accepted` for pending Google one-time verification is the right API shape, once the product type normalization issue is addressed.

## Verification run during review

- `cargo check` passed.
- `cargo test verify_purchase` passed: 4 tests passed.
