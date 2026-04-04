# Bridge Behavioral Spec Gap Review

Date: 2026-04-01
Updated: 2026-04-04
Source spec: `docs.notes/BEHAVIORAL_SPEC.md`
Codebase reviewed: `src/`, `migrations/`

## Scope

This document compares the current Bridge implementation against the behavioral spec and focuses on real behavior in code, not intent or TODO comments.

Status labels:

- `Fixed`: current code now matches the relevant spec requirement for this item.
- `Partial`: the main path exists, but required behaviors, fields, or safety checks are missing.
- `Gap`: missing, broken, or contradicted by the current implementation.

## Executive Summary

The main gaps are in correctness and contract fidelity:

- `verify-purchase` is much thinner than the spec and misses several required behaviors.
- There are schema/runtime mismatches where handlers write columns that do not exist.
- webhook ordering and retry behavior is not strong enough to satisfy the spec's stale-event guarantees.
- the remaining security-sensitive gap is narrower now: admin auth and per-API-key rate limiting are fixed, but agent charge flow still does not bind the request endpoint back to the token as the spec describes.

## Highest-Risk Gaps

### 1. Runtime schema mismatches

These are not just spec gaps; they look like live failure paths.

- `src/handlers/verify_purchase.rs` queries and inserts `pay.fraud_prevention.purchase_token`, but `migrations/08_create_fraud_prevention_table.sql` does not define a `purchase_token` column. The same insert also omits the required `provider` column.
- `src/handlers/subscriptions_actions.rs` updates `pay.subscriptions.acknowledged_at`, but `migrations/03_create_subscriptions_table.sql` does not define that column. `acknowledged_at` exists on `pay.payments` instead (`migrations/04_create_payments_table.sql`).
- `src/handlers/subscriptions_actions.rs` updates `price_step_up_pending`, but the schema only defines Google price-step-up fields such as `google_requires_price_step_up_consent` and `google_price_step_up_consent_status`.

### 2. `verify-purchase` is far from the spec

Spec sections 5, 50, 52, and 53 require more than the current handler does.

- `src/handlers/verify_purchase.rs` accepts only `external_user_id`, `provider`, `subscription_id`, and `purchase_token`. The spec also requires `product_type`, plus different behavior for subscriptions vs one-time products.
- The handler does not record a payment in `pay.payments` during verification.
- The handler does not persist the verified `purchase_token` into `pay.subscriptions`.
- The handler does not populate `amount_cents` in the response.
- Google Play linking flows from the spec are missing: no `LinkingRequired` response path, no resubscription linking, no obfuscated-account recovery.
- Google Play acknowledgement happens inside `verify_google_play`, but it is not tracked through `payments.acknowledged_at` as the spec requires.
- The callback emitted after verification is `subscription.verified`, while the spec expects `subscription.activated` or `purchase.one_time` depending on purchase type.

### 3. Webhook ordering and retry are not safe enough

- `src/db/subscriptions.rs::update_subscription_status` has an event-time guard, but `src/db/subscriptions.rs::upsert_subscription_tx` does not. Activation and renewal events in `src/webhooks/processor.rs` can therefore overwrite newer states with older events.
- `src/webhooks/scheduler.rs::retry_webhooks` calls `process_webhook()` again when retrying delivery. That means a delivery retry can re-run DB mutations instead of being forward-only.

### 4. Security-sensitive gaps

- Partial: `src/db/agent.rs::charge_agent` now scopes token consumption to the same app and user, but the API still does not accept or verify the request `endpoint` against the token as required by spec section 41.

## Section Review

### Rate Limiting

| Spec area | Status | Notes |
|---|---|---|
| 3.2 Default endpoint limits | Partial | Defaults are present, but `/purchase/register` is routed in `src/main.rs` while the limiter checks for `/purchases/register`, so purchase registration misses the intended endpoint group. |
| 3.3 Per-IP unauthenticated limits | Gap | No middleware exists for failed-auth or unauthenticated per-IP limits. |


### Core API Flows

| Spec area | Status | Notes |
|---|---|---|
| 4. Checkout Flow | Gap | `email` is optional with fake email fallback, Google Play mobile checkout not implemented, Coinbase is rejected, metadata/redirect handling not aligned with spec. |
| 5. Purchase Verification | Gap | Major behavioral gap; see Highest-Risk item 2. |
| 6. Purchase Registration | Gap | Request `reason` is unused and flow stops at placeholder. |
| 7. Subscription Queries | Gap | Single-item response does not return provider-specific fields the spec calls for. |
| 8. Subscription Cancellation | Gap | Uses JSON body `external_user_id` instead of query params, ignores provider disambiguation, missing revocation metadata for immediate cancel. |
| 9. Subscription Resume | Gap | Body-based user lookup (should be query param), no provider query param. |
| 10. Billing Portal | Gap | Only works where `provider_customer_id` exists, only implemented for Creem. |
| 11. Payment History | Gap | Response omits `provider` and `provider_transaction_id`, uses `amount` instead of `amount_cents`. |

### Webhook Ingress and Processing

| Spec area | Status | Notes |
|---|---|---|
| 12. Webhook Ingress | Gap | App-not-found errors not silent 404, provider header names differ, config from `provider_configs` not app-level secrets. |
| 13-31. Canonical webhook processing | Gap | Several flows simplified to status-only updates, do not persist all spec-mandated fields or reasons. |
| 32-37. Google Play special cases | Gap | Price-step-up accept/decline API handlers schema-mismatched. |
| 38. Callback Forwarding | Gap | No explicit 10-second timeout, no dead-letter state, retry reprocesses webhook instead of forward-only delivery. |

### Specific Webhook Gaps Worth Calling Out

- `src/webhooks/processor.rs` handles `subscription.updated` by writing raw status when present, but the spec expects a more controlled normalized update path.
- `src/webhooks/processor.rs` only uses the stale-event guard for some update paths. Activation and renewal still use the unguarded upsert helper.

### Agent 402

| Spec area | Status | Notes |
|---|---|---|
| 40. Token Creation | Gap | Does not validate email format, endpoint support, or amount; does not ensure `agent_credits` row exists. |
| 41. Token Charge | Partial | `src/db/agent.rs::charge_agent` now binds token use to the same app and user, but the request still does not carry or verify `endpoint`, so it is not fully at spec. |

### GDPR and Data Retention

| Spec area | Status | Notes |
|---|---|---|
| 44. User Anonymization | Gap | No separate app callback sent on anonymization. |
| 45. Data Export | Gap | Does not include webhook records, agent credits, or agent transactions. |

### Background Jobs

| Spec area | Status | Notes |
|---|---|---|
| 46. Reconciliation | Gap | Runs for all providers not just Google/Apple, emits `admin.drift_alert` not `reconciliation.drift_detected`. |
| 47. Price Step-Up Expiry | Gap | Callback does not include richer reason/context described by spec. |

### DB Behaviors

| Spec area | Status | Notes |
|---|---|---|
| 50. Payment Recording | Gap | Many webhook callers ignore error result from `record_payment_tx`. |
| 52. Subscription Store/Activate | Gap | Lacks `last_event_time` guard on conflict updates. |

## Recommended Fix Order

1. Fix schema/runtime mismatches first.
2. Bring `verify-purchase` up to the spec contract.
3. Separate webhook processing from webhook delivery retries.
4. Add stale-event guards to all subscription mutation paths, especially activation/renewal upserts.
5. Finish agent token endpoint scoping so charge requests prove the token is for the requested endpoint, not just the same app and user.
6. Tighten API response shapes to match the spec exactly.

## Bottom Line

Bridge is structurally close to the target system, but it is not yet behaviorally aligned with the spec in several critical places. The biggest problems are not missing routes; they are correctness gaps around verification, schema drift, webhook replay/order safety, and auth/scoping.
