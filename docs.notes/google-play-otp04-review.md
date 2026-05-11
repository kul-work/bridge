# Google Play OTP-04 Reshape Review

Date: 2026-05-10

Scope: staged Rust and OTP-04 harness changes for the Google Play one-time product pending/approval flow. This is review advice only; no code changes were made as part of the review.

## Findings

### 2. Blocker/design risk: OTP success can be downgraded back to pending

The OTP-04 flow can currently regress:

1. `/verify-purchase` sees a slow-card token and stores payment `pending`.
2. Google RTDN/webhook marks the payment `success`.
3. A later verify retry still sees `purchaseState: 2` and writes `pending` again.

The DB payment upsert unconditionally applies the incoming status for the same user, so `success` can be overwritten by `pending`. The shell test avoids this by checking DB before retrying verify, but the application should enforce monotonic payment state itself.

Advice: make one-time payment status monotonic in the source-of-truth write path. At minimum, do not allow terminal/stronger states like `success`, `refunded`, or `cancelled` to regress to `pending`.



