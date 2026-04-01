# Bridge Behavioral Spec Gap Review

Date: 2026-04-01
Source spec: `docs.notes/BEHAVIORAL_SPEC.md`
Codebase reviewed: `src/`, `migrations/`

## Scope

This document compares the current Bridge implementation against the behavioral spec and focuses on real behavior in code, not intent or TODO comments.

Status labels:

- `Partial`: the main path exists, but required behaviors, fields, or safety checks are missing.
- `Gap`: missing, broken, or contradicted by the current implementation.

## Executive Summary

Bridge already has the broad skeleton the spec describes: startup, API key auth, checkout idempotency, subscription queries, webhook ingress, webhook forwarding, agent endpoints, GDPR endpoints, and background jobs all exist.

The main gaps are in correctness and contract fidelity:

- `verify-purchase` is much thinner than the spec and misses several required behaviors.
- There are schema/runtime mismatches where handlers write columns that do not exist.
- webhook ordering and retry behavior is not strong enough to satisfy the spec's stale-event guarantees.
- admin auth and agent token charging are materially weaker than the spec's security model.

## Highest-Risk Gaps

### 1. Runtime schema mismatches

These are not just spec gaps; they look like live failure paths.

- `src/handlers/verify_purchase.rs` queries and inserts `pay.fraud_prevention.purchase_token`, but `migrations/08_create_fraud_prevention_table.sql` does not define a `purchase_token` column. The same insert also omits the required `provider` column.
- `src/handlers/subscriptions_actions.rs` updates `pay.subscriptions.acknowledged_at`, but `migrations/03_create_subscriptions_table.sql` does not define that column. `acknowledged_at` exists on `pay.payments` instead (`migrations/04_create_payments_table.sql`).
- `src/handlers/subscriptions_actions.rs` updates `price_step_up_pending`, but the schema only defines Google price-step-up fields such as `google_requires_price_step_up_consent` and `google_price_step_up_consent_status`.
- `src/db/agent.rs` uses `ON CONFLICT (app_id, charge_id)` in `apply_topup_if_new`, but `migrations/07_create_agent_credits_tables.sql` does not define a matching unique index or constraint on `pay.agent_transactions`.

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
- `src/webhooks/processor.rs` only discards unresolved orphan events for Creem. For other providers, unresolved events can still produce forwarded callbacks with no resolved `external_user_id`, which is weaker than spec section 53.

### 4. Security-sensitive gaps

- `src/middleware/admin_auth.rs` currently treats any bearer token as sufficient for admin access. The spec expects Tyde internal Clerk org verification.
- `src/db/agent.rs::charge_agent` consumes an agent token by `token_id` only. It does not verify that the token belongs to the same app, user, or endpoint requested by the caller.
- `src/middleware/rate_limit.rs` applies limits per app/group, not per API key as required by spec section 3.1.

## Section Review

### Startup, Auth, Health, Admin

| Spec area | Status | Notes |
|---|---|---|
| 1. Startup & Initialization | Partial | `src/main.rs` starts tracing, DB, routes, and background jobs. Gap: provider secrets are loaded from `pay.provider_configs`, not decrypted from `apps`, and `master_encryption_key` is read in `src/config.rs` but not used. |
| Admin UI security (part of section 1) | Gap | `src/middleware/admin_auth.rs` only checks for header presence; it does not verify JWT signature or Clerk organization membership. |

### Rate Limiting

| Spec area | Status | Notes |
|---|---|---|
| 3.1 Per-API-Key limits | Partial | `src/middleware/rate_limit.rs` supports endpoint groups and app overrides, but the key is `app_id + group`, not API key + group. |
| 3.2 Default endpoint limits | Partial | Defaults are present, but `/purchase/register` is routed in `src/main.rs` while the limiter checks for `/purchases/register`, so purchase registration misses the intended endpoint group. |
| 3.3 Per-IP unauthenticated limits | Gap | No middleware exists for failed-auth or unauthenticated per-IP limits. |


### Core API Flows

| Spec area | Status | Notes |
|---|---|---|
| 4. Checkout Flow | Partial | `src/handlers/checkout.rs` supports idempotency and provider config lookup. Gaps: `email` is optional, fake email fallback is generated, Google Play mobile checkout is not implemented, Coinbase is rejected, and metadata/redirect handling is not fully aligned with the spec. |
| 5. Purchase Verification | Gap | Major behavioral gap; see Highest-Risk item 2. |
| 6. Purchase Registration | Partial | `src/handlers/payments.rs::register_purchase` creates a pending placeholder as expected, but the request `reason` is unused and the flow stops at the placeholder. |
| 7. Subscription Queries | Partial | Listing and single-item fetch exist in `src/handlers/subscriptions.rs`, with keyset pagination for list. Gap: the single-item response does not return provider-specific fields the spec calls for. |
| 8. Subscription Cancellation | Partial | `src/handlers/subscriptions_actions.rs::cancel_subscription` exists, but it uses JSON body `external_user_id` instead of required query params, ignores provider disambiguation during lookup, and does not set all spec fields such as revocation metadata for immediate cancel. |
| 9. Subscription Resume | Partial | Present, but same contract mismatch as cancellation: body-based user lookup, no provider query param, simplified DB update. |
| 10. Billing Portal | Partial | Present in `src/handlers/subscriptions_actions.rs`, but only works where `provider_customer_id` exists and `src/services/provider_api.rs` only implements billing portal creation for Creem. |
| 11. Payment History | Partial | `src/handlers/payments.rs::get_payments` paginates correctly, but the response omits `provider` and `provider_transaction_id`, and uses `amount` instead of `amount_cents`. |

### Webhook Ingress and Processing

| Spec area | Status | Notes |
|---|---|---|
| 12. Webhook Ingress | Partial | `src/webhooks/ingress.rs` implements provider-specific signature verification and dedup insert. Gaps: app-not-found errors are not the spec's silent 404 shape, provider header names differ in some cases, and provider config comes from `provider_configs` rather than encrypted app-level secrets. |
| 13-31. Canonical webhook processing | Partial | `src/webhooks/processor.rs` handles many canonical events, but several flows are simplified to status-only updates and do not persist all spec-mandated fields or reasons. |
| 32-37. Google Play special cases | Partial | Price step-up, deferred, pause-scheduled, and informational events exist, but some only log or update one field, and the price-step-up accept/decline API handlers are schema-mismatched. |
| 38. Callback Forwarding | Partial | `src/webhooks/forwarding.rs` builds canonical JSON and HMAC headers, and retries up to 3 times through `webhook_delivery`. Gaps: no explicit 10-second timeout, no dead-letter state beyond attempt count, and retry reprocesses the webhook instead of forward-only delivery. |

### Specific Webhook Gaps Worth Calling Out

- `src/webhooks/processor.rs` maps Coinbase `charge:failed` to `charge.failed`, but there is no handler branch for `charge.failed`. Spec section 43 says this event should at least log a warning.
- `src/webhooks/processor.rs` handles `refund.created` by updating payment status, but it does not normalize to the same callback contract as `payment.refunded`.
- `src/webhooks/processor.rs` handles `subscription.updated` by writing raw status when present, but the spec expects a more controlled normalized update path.
- `src/webhooks/processor.rs` only uses the stale-event guard for some update paths. Activation and renewal still use the unguarded upsert helper.

### Agent 402

| Spec area | Status | Notes |
|---|---|---|
| 40. Token Creation | Partial | `src/handlers/agent.rs::token` creates expiring tokens, but it does not validate email format, endpoint support, or amount, and it does not ensure an `agent_credits` row exists before token creation. |
| 41. Token Charge | Gap | `src/db/agent.rs::charge_agent` does not verify that the token belongs to the same app/user/endpoint. The spec requires strict token scoping and ownership checks. |
| 42. Charge Confirmed (Coinbase) | Partial | `src/webhooks/processor.rs` applies topups idempotently in intent, but the underlying `ON CONFLICT` target in `src/db/agent.rs::apply_topup_if_new` does not match the schema. |
| 43. Charge Failed (Coinbase) | Gap | The event is normalized, but no explicit handler branch logs it as required. |

### GDPR and Data Retention

| Spec area | Status | Notes |
|---|---|---|
| 44. User Anonymization | Partial | `src/handlers/users.rs` and `src/db/users.rs` cancel active subscriptions and scramble `external_user_id`. This aligns with most of the spec, but provider-side cancellation coverage depends on provider support and no separate app callback is sent. |
| 45. Data Export | Partial | `src/handlers/users.rs::data_export` returns subscriptions and payments only. It does not include webhook records, agent credits, or agent transactions. |

### Background Jobs

| Spec area | Status | Notes |
|---|---|---|
| 46. Reconciliation | Partial | `src/webhooks/scheduler.rs::reconcile_subscriptions` exists, but it runs for all enabled apps/providers, not just the spec's Google/Apple focus, and it emits `admin.drift_alert` rather than `reconciliation.drift_detected`. |
| 47. Price Step-Up Expiry | Partial | Present and auto-cancels expired consent cases, but the forwarded callback does not include the richer reason/context described by the spec. |

### DB Behaviors

| Spec area | Status | Notes |
|---|---|---|
| 50. Payment Recording | Partial | `src/db/payments.rs::record_payment_tx` mostly matches the spec's fraud-guarded UPSERT, but many webhook callers ignore its error result. |
| 52. Subscription Store/Activate | Partial | `src/db/subscriptions.rs::upsert_subscription_tx` increments version and updates fields, but lacks a `last_event_time` guard on conflict updates. |
| 53. User Lookup Strategies | Partial | Strategies 1, 2A, 2B, 3, and 4 exist in `src/webhooks/processor.rs::resolve_user`, but unresolved non-Creem events are not discarded as the spec requires. |

## Recommended Fix Order

1. Fix schema/runtime mismatches first.
2. Bring `verify-purchase` up to the spec contract.
3. Separate webhook processing from webhook delivery retries.
4. Add stale-event guards to all subscription mutation paths, especially activation/renewal upserts.
5. Harden admin auth and agent token scoping.
6. Tighten API response shapes to match the spec exactly.

## Bottom Line

Bridge is structurally close to the target system, but it is not yet behaviorally aligned with the spec in several critical places. The biggest problems are not missing routes; they are correctness gaps around verification, schema drift, webhook replay/order safety, and auth/scoping.