# Behavioral Spec Alignment Audit

Date: 2026-04-06

## Scope

Compared [docs.notes/BEHAVIORAL_SPEC.md](./BEHAVIORAL_SPEC.md) against the current Bridge implementation in `src/` and the supporting schema in `migrations/`.

Status labels used below:

- `Aligned`: behavior is implemented closely enough to the spec.
- `Partial`: the flow exists, but one or more contract details differ.
- `Diverged`: the implementation intentionally or materially behaves differently.
- `Missing`: the spec describes behavior that is not implemented.

The spec itself is marked `Proposal / Under Review`, so some items below are probably stale-spec issues rather than code bugs. Where that seems likely, it is called out directly.

## Executive Summary

High-confidence mismatches worth attention first:

1. Provider credential storage/encryption does not match the spec. Code reads plaintext provider config from `pay.provider_configs`, while the spec repeatedly describes encrypted credentials loaded from `apps` via an encryption key.
2. The documented per-endpoint default rate limits are not what the code actually enforces. In practice, the app-wide default of `120/min` wins unless JSON overrides are configured.
3. Unauthenticated IP rate limiting ignores `X-Forwarded-For` and `X-Real-IP`, so deployed behavior behind a proxy will drift from the spec.
4. Webhook ingress is looser than the spec for invalid tokens and missing provider event IDs.
5. Apple support is only partial. Checkout has an Apple mobile stub, but verification, webhook ingress, and reconciliation support are not implemented.
6. `verify-purchase` does not currently record provider amounts even though the spec treats `amount_cents` as part of the verification result.
7. Agent token creation semantics differ materially from the documented contract.
8. GDPR anonymization and data export behavior diverge from the spec in user-visible ways.

## Detailed Findings

| Priority | Spec Section(s) | Status | Finding | Code Evidence |
|---|---|---|---|---|
| P0 | 1, 4, 12 | Diverged | The spec says provider credentials are Bridge-owned settings decrypted from DB state using an encryption key and describes them as living in the `apps` table. The implementation reads raw JSON from `pay.provider_configs`, and `MASTER_ENCRYPTION_KEY` is loaded but never used. This looks like either an unfinished security feature or stale documentation. | `src/config.rs:11-16`, `src/config.rs:46`, `src/db/provider_configs.rs:15-29`, `src/handlers/checkout.rs:76-103`, `src/handlers/verify_purchase.rs:149-156`, `src/webhooks/ingress.rs:27-29`, `migrations/11_create_provider_configs_table.sql:13-14`, `migrations/11_create_provider_configs_table.sql:73` |
| P0 | 3 | Diverged | The spec defines lower default limits for endpoint groups like checkout and verify-purchase. The middleware instead prefers `apps.api_rate_limit_per_minute` before the hardcoded endpoint defaults, and the schema default is `120`, so checkout/verify/purchase registration usually run at `120/min` unless app JSON overrides are explicitly configured. | `src/middleware/rate_limit.rs:120-130`, `src/middleware/rate_limit.rs:179-197`, `migrations/01_create_apps_table.sql:22` |
| P0 | 3.3 | Partial | Spec IP extraction order is `X-Forwarded-For` -> `X-Real-IP` -> connection IP. The middleware only uses `ConnectInfo<SocketAddr>`, so proxied deployments will not follow the documented rule. | `src/middleware/rate_limit.rs:113-118` |
| P0 | 12 | Partial | Invalid webhook tokens are supposed to fail silently with `404`. Unknown app tokens do return `404`, but malformed UUID path tokens return a validation error instead. | `src/webhooks/ingress.rs:20-24`, `src/webhooks/ingress.rs:175-177`, `src/webhooks/ingress.rs:277-279`, `src/webhooks/ingress.rs:375-377` |
| P0 | 12 | Diverged | Missing provider event IDs are supposed to return `400`. The ingress handlers usually fall back to the string `"unknown"` instead of rejecting the webhook, which weakens deduplication and audit quality. | `src/webhooks/ingress.rs:96-100`, `src/webhooks/ingress.rs:221-222`, `src/webhooks/ingress.rs:321-322`, `src/webhooks/ingress.rs:420-421` |
| P1 | 4, 5, 12, 46 | Missing | Apple support is only partially present. Checkout can return Apple mobile metadata, but there is no Apple verify-purchase path, no Apple webhook ingress route, and no provider API/reconciliation support. | `src/handlers/checkout.rs:279-308`, `src/handlers/verify_purchase.rs:443-473`, `src/webhooks/mod.rs:15-21`, `src/services/provider_api.rs:13-107`, `src/services/provider_api.rs:111-179`, `src/services/provider_api.rs:254-352` |
| P1 | 5 | Partial | Spec says verify-purchase should record and return `amount_cents` from the provider response. Current provider verification paths set `amount_cents` to `None`, and the write path falls back to `0`, so the data is not actually captured. | `src/handlers/verify_purchase.rs:266-275`, `src/handlers/verify_purchase.rs:559-566`, `src/handlers/verify_purchase.rs:612-620`, `src/handlers/verify_purchase.rs:667-675`, `src/handlers/verify_purchase.rs:821-828`, `src/handlers/verify_purchase.rs:860-867` |
| P1 | 5 | Diverged | The spec describes Google Play resubscribe linking that can resolve the original user via expired obfuscated identifiers and continue the flow. The current implementation returns `linking_required` when the obfuscated identifier does not match the supplied user instead of rebinding to the original user. | `src/handlers/verify_purchase.rs:281-319`, `src/handlers/verify_purchase.rs:769-840` |
| P1 | 6 | Partial | Purchase registration exists, but the response contract differs. The spec says `{"status":"registered"}`; code returns `201` with `{ success, message, subscription_id, status }`, where `status` is the DB state (`pending`). | `src/handlers/payments.rs:197-228`, `src/db/subscriptions.rs:195-219` |
| P1 | 7 | Partial | Subscription list pagination is implemented, but the default `limit` is `10` instead of the documented `20`. | `src/handlers/subscriptions.rs:82` |
| P1 | 8 | Partial | Scheduled cancellation keeps the subscription active in DB, which matches the lifecycle intent, but the API response differs from the documented contract. The spec says the response status is `"cancelled"`; code returns the row's actual status, which is typically still `active` for scheduled cancellation. | `src/handlers/subscriptions_actions.rs:85-111`, `src/handlers/subscriptions_actions.rs:142-151` |
| P1 | 9 | Partial | Resume behavior exists, but the response contract differs. The spec expects `{ "status": "active", "subscription_id": "..." }`; code returns `{ success, message }`. | `src/handlers/subscriptions_actions.rs:156-225` |
| P1 | 12 | Partial | The spec names Creem's signature header as `Webhook-Signature`. The code verifies `x-signature`. Header names are case-insensitive, but the actual header key is still a spec/code mismatch that should be resolved in one direction. | `docs.notes/BEHAVIORAL_SPEC.md:368`, `src/webhooks/ingress.rs:194`, `src/webhooks/ingress.rs:296` |
| P1 | 20 | Partial | Cancellation-scheduled callbacks are forwarded as `subscription.cancelled`, but the code explicitly overrides callback status to `active` so the event type and status no longer match each other. The spec describes a cancelled event carrying `current_period_end`; it does not describe this mixed payload. | `src/webhooks/processor.rs:1121-1152` |
| P1 | 40 | Diverged | Agent token creation differs materially from the documented contract. The spec says to UPSERT a zero-balance account if missing and return `{ token, expires_in_seconds }`. The implementation requires an existing credits row, requires a `nonce`, does not validate email format or endpoint allowlist, and returns `{ token_id, amount_cents, expires_at }`. | `src/handlers/agent.rs:17-25`, `src/handlers/agent.rs:55-88`, `src/db/agent.rs:73-104` |
| P1 | 41 | Partial | Agent charge is atomic and scoped to user/endpoint, but the request/response contract differs from the spec: code accepts `token_id` instead of `token` and returns `new_balance_cents` in addition to the charge result. | `src/handlers/agent.rs:27-37`, `src/handlers/agent.rs:90-109`, `src/db/agent.rs:160-218` |
| P1 | 44 | Diverged | The spec explicitly says Bridge should not send separate callbacks for anonymization-triggered cancellations. The implementation spawns a direct `user.anonymized` webhook anyway, and it bypasses the normal `webhook_delivery` retry/dead-letter pipeline. | `src/handlers/users.rs:33-45`, `src/handlers/users.rs:123-168` |
| P1 | 45 | Partial | Data export is supposed to return all Bridge data for the user. The current implementation hard-caps subscriptions/payments at 100 rows and uses `unwrap_or_default()` for several queries, which can silently omit data on DB errors. | `src/handlers/users.rs:60-116`, `src/db/payments.rs:159-179`, `src/db/subscriptions.rs:101-121` |
| P1 | 46 | Partial | Reconciliation exists, but the implementation differs from the spec in two ways: it runs for whatever providers have config rather than only Google Play/Apple, and it logs an admin alert message instead of sending an admin alert email. Apple is also still unsupported. | `src/webhooks/scheduler.rs:93-143`, `src/webhooks/scheduler.rs:155-201`, `src/services/provider_api.rs:254-352` |
| P2 | 1 | Partial | Startup/background workers largely match the spec, but the implementation also starts a webhook retry worker that the startup section does not mention explicitly. This is not harmful, but it means the spec is incomplete about active background jobs. | `src/main.rs:82-89` |
| P2 | 4 | Partial | Checkout behavior is mostly aligned, but the implementation has an extra alias route (`/api/v1/payment/checkout`) that the spec does not document. | `src/main.rs:97-99` |
| P2 | 54 | Partial | Health behavior is aligned, but the code also exposes `/api/v1/health` in addition to `/health`. | `src/main.rs:142-145`, `src/handlers/mod.rs:11-16` |

## Section Status Matrix

| Spec Section | Status | Notes |
|---|---|---|
| 1. Startup & Initialization | Partial | Core startup behavior exists; credential encryption path in spec is not implemented. |
| 2. API Key Authentication | Aligned | Header extraction, key prefix lookup, hash verification, app enabled checks, and `last_used_at` update are present. |
| 3. Rate Limiting | Partial | Middleware exists, but endpoint defaults and proxy IP extraction do not match spec. |
| 4. Checkout Flow | Partial | Core checkout is implemented; Apple is only a metadata stub and the code exposes an extra alias route. |
| 5. Purchase Verification | Partial | Main flow exists, but amount capture and Google relinking semantics diverge; Apple is missing. |
| 6. Purchase Registration | Partial | Placeholder registration exists; response contract differs. |
| 7. Subscription Queries | Partial | Query behavior is implemented; default page size differs. |
| 8. Subscription Cancellation | Partial | Cancel flow exists; scheduled-cancel response contract differs. |
| 9. Subscription Resume | Partial | Resume flow exists; response contract differs. |
| 10. Billing Portal | Aligned | Implemented for supported providers and guarded by `provider_customer_id`. |
| 11. Payment History | Aligned | Query and pagination are implemented. |
| 12. Webhook Ingress | Partial | Signature verification and async processing exist; invalid-token and missing-event-id semantics differ. |
| 13. Subscription Activation | Aligned | Main mutation/payment/callback path is implemented. |
| 14. Subscription Pending | Aligned | Pending is stored and forwarding is suppressed. |
| 15. Grace Period | Aligned | Past-due transition and callback are implemented. |
| 16. Subscription Revoked | Aligned | Revocation, refund guard, and callback are implemented. |
| 17. Subscription On Hold | Aligned | Stale-event guard and callback are implemented. |
| 18. Subscription Paused | Aligned | State guard and callback are implemented. |
| 19. Subscription Restarted | Aligned | Resume-from-paused flow is implemented. |
| 20. Cancellation Scheduled | Partial | DB change exists, but callback payload semantics differ from spec. |
| 21. Subscription Expired / Inactive | Aligned | Expiry handling and callback are present. |
| 22. Subscription Cancelled | Aligned | Cancelled transition and callback are present. |
| 23. Order Created | Aligned | Pending payment recording is present. |
| 24. Order Failed | Aligned | Failed payment recording and subscription notification flag are present. |
| 25. One-Time Product Purchased | Aligned | Payment recording and callback are present. |
| 26. One-Time Product Cancelled | Aligned | Cancellation idempotency and callback are present. |
| 27. Purchase Voided / Refund | Aligned | Refund updates and revoke flow are present. |
| 28. Pending Purchase Cancelled | Aligned | Cancel transition and callback are present. |
| 29. Dispute Created | Aligned | Admin alert path and app callback exist. |
| 30. Refund Created | Aligned | Mapped into refund handling. |
| 31. Subscription Updated | Aligned | Normalized upsert and event remapping exist. |
| 32. Price Step-Up Consent | Aligned | State storage and callback are present. |
| 33. Subscription Deferred | Aligned | Deferred-until storage is present. |
| 34. Pause Scheduled | Aligned | Schedule storage and background application are present. |
| 35. Price Changed | Aligned | Audit payment write and callback path exist. |
| 36. Price Change Updated | Aligned | Informational event is handled. |
| 37. Expired Voided | Aligned | Informational event is handled. |
| 38. Callback Forwarding | Aligned | HMAC signing, timeout, retries, dead-lettering, and stale-forward suppression are implemented. |
| 39. Agent Balance Query | Aligned | Returns current balance and lifetime spent. |
| 40. Agent Token Creation | Diverged | Contract differs materially from the spec. |
| 41. Agent Charge | Partial | Atomic reserve exists; API contract differs. |
| 42. Charge Confirmed | Aligned | Idempotent top-up application is implemented. |
| 43. Charge Failed | Aligned | Logged without mutation. |
| 44. User Anonymization | Diverged | Core anonymization exists, but the spec explicitly forbids extra callbacks and code sends one. |
| 45. Data Export | Partial | Export exists, but completeness guarantees are weaker than spec. |
| 46. Reconciliation | Partial | Job exists; provider scope and admin alert behavior differ, Apple still missing. |
| 47. Price Step-Up Expiry | Aligned | Scheduled auto-cancel flow is implemented. |
| 48. Pause Scheduler | Aligned | Scheduled pause and orphan cleanup are implemented. |
| 49. Webhook Log Cleanup | Aligned | Cleanup worker calls the retention functions. |
| 50. Payment Recording | Aligned | UPSERT + fraud guard match the documented DB behavior closely. |
| 51. Webhook Deduplication | Aligned | Primary and secondary dedup behaviors are present. |
| 52. Subscription Store / Activate | Aligned | UPSERT, versioning, and `last_event_time` stale-event guard are implemented. |
| 53. User Lookup Strategies | Aligned | Strategies 1-4 plus Creem orphan guard are implemented; no email fallback. |
| 54. Health Check | Aligned | `GET /health` matches spec; extra `/api/v1/health` route also exists. |

## Likely Stale-Spec Areas

These look more like documentation drift than broken code, but they should still be resolved so the spec can be used safely as an implementation contract:

- The spec still says provider credentials come from the `apps` table, while the implementation and schema have clearly standardized on `pay.provider_configs`.
- The spec mentions `ENCRYPTION_KEY`, while code/config use `MASTER_ENCRYPTION_KEY` and do not currently apply encryption/decryption at all.
- Some response shapes in the spec (`register_purchase`, `resume_subscription`, `agent/token`, `agent/charge`) no longer match the actual API.
- The spec mentions Apple in multiple places, but the codebase is only partially prepared for it.

## Suggested Next Actions

1. Decide whether the source of truth is the spec or the shipped code for provider config storage and encryption. That decision affects both security work and a large amount of doc cleanup.
2. Fix or explicitly bless the rate-limit behavior. Right now the documented endpoint defaults are misleading.
3. Tighten webhook ingress semantics for malformed tokens and missing event IDs if the spec is intended to be exact.
4. Decide whether Apple support should be removed from the spec for now or completed in code.
5. Resolve the GDPR mismatch around `user.anonymized` callbacks before other apps build against the current behavior.
