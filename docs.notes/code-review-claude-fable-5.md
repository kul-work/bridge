# Bridge Code Review — Bug Hunt

**Reviewer:** claude-fable-5
**Date:** 2026-07-05
**Scope:** Full `src/` (~22k LOC Rust), `migrations/`, and test suites.
**Method:** 7 parallel subagent reviews across subsystems, followed by direct source verification of every High/Critical finding.

All findings below were confirmed by reading the actual code (line numbers verified). Style, naming, and formatting issues were excluded per request. Items I could not prove are under **Needs Verification**.

---

## Severity summary

| # | Severity | Area | One-line |
|---|----------|------|----------|
| 1 | High | GP JWT auth | Pub/Sub JWT accepted without verifying service-account `email` / `email_verified` |
| 7 | High | Scheduler | Terminal (cancelled/revoked) subs can be flipped to `paused` |
| 8 | High | Scheduler | Scheduler callbacks lost if enqueue fails after state mutation |
| 10 | Medium | GP webhook | Test notifications ACKed with no durable inbox/suppressed row |
| 12 | Medium | DB constraint | Global `purchase_token UNIQUE` breaks app isolation |
| 13 | Low | Email | Poisoned mutex `.expect()` panics production email path |
| 14 | Low | Rate limit | Spoofable `X-Forwarded-For` + unbounded key growth |

---

## 1. High — Pub/Sub JWT accepted without verifying issuer identity (`email` / `email_verified`)

**File:** [src/services/google_play/client.rs#L674-L693](file:///c%3A/share/tyde/bridge/src/services/google_play/client.rs#L674-L693) — `verify_jwt_with_jwk` / `verify_pubsub_signature`

**What is wrong.** The production JWT path verifies the RS256 signature against Google's JWKs, checks `iss = https://accounts.google.com`, `exp`, and (optionally) `aud`. It never verifies that the token's `email` claim equals the configured Pub/Sub push service account, nor that `email_verified == true`. `PubSubClaims` even carries `email: Option<String>` but it is only logged, never compared.

**Why it's a real bug.** Google's authenticated-push contract requires validating **both** `aud` and the service-account `email`/`email_verified`. Any Google-signed OIDC token minted for the same audience — obtainable by other GCP identities — passes. This is a signature/authenticity control on a money-mutating ingress.

**Failure scenario.** A GCP identity (misconfigured internal service, or an attacker able to mint an OIDC token for the Bridge audience) POSTs a crafted RTDN body. Bridge accepts it because it is Google-signed and audience-matched, then processes fake cancellation/revocation/price-change events.

**Smallest safe fix.** Add an expected Pub/Sub service-account email to provider/client config. Extend `PubSubClaims` with `email_verified: Option<bool>`. After signature+audience checks, require `claims.email.as_deref() == Some(expected)` and `claims.email_verified == Some(true)`. Fail closed when verification is enabled but no expected email is configured.

**Regression test.** Yes — accept when all claims match; reject on `email` mismatch; reject when `email_verified` missing/false.

---

## 7. High — Scheduled pause can overwrite terminal (cancelled/revoked) subscriptions

**File:** [src/db/subscriptions.rs#L705-L730](file:///c%3A/share/tyde/bridge/src/db/subscriptions.rs#L705-L730) — `list_pending_pause_subscriptions`; [#L888-L913](file:///c%3A/share/tyde/bridge/src/db/subscriptions.rs#L888-L913) — `mark_subscription_paused`; caller [src/webhooks/scheduler.rs#L894-L917](file:///c%3A/share/tyde/bridge/src/webhooks/scheduler.rs#L894-L917)

**What is wrong.** Selection uses only `google_pause_scheduled_at <= NOW() AND status != 'paused'`; the update sets `status='paused'` for any non-paused row. Neither excludes terminal states (`cancelled`, `expired`, `revoked`, `replaced`). The monotonicity guard is applied only to `last_event_time` (via `CASE`), not to the status write, and the caller passes `now_ms` rather than the scheduled event time, so the pause almost always looks "newer".

**Why it's a real bug.** Violates the monotonic-transition invariant and terminal-state integrity: a stale scheduled pause can resurrect a cancelled/revoked subscription to `paused` and emit a false `subscription.paused` callback.

**Failure scenario.** Pause scheduled for tomorrow → Google sends revoke today → Bridge sets `revoked` but the stale `google_pause_scheduled_at` remains → scheduler runs tomorrow → the revoked row is selected and flipped to `paused`.

**Smallest safe fix.** Restrict both the SELECT and the UPDATE to pausable statuses (`'active','trial','past_due','on_hold'`) and add a monotonic guard on status. Prefer using the scheduled-pause timestamp (not `now_ms`) as the transition time. Clearing `google_pause_scheduled_at` on terminal transitions is a good defense-in-depth follow-up.

**Regression test.** Yes — a row with `google_pause_scheduled_at <= NOW()` and status `revoked`/`cancelled` must not become `paused`, and no paused callback is enqueued.

---

## 8. High — Scheduler state transitions can lose their callback (no durable work item)

**File:** [src/webhooks/scheduler.rs#L794-L870](file:///c%3A/share/tyde/bridge/src/webhooks/scheduler.rs#L794-L870) — `process_price_step_up_expiry`; [#L894-L953](file:///c%3A/share/tyde/bridge/src/webhooks/scheduler.rs#L894-L953) — `process_pause_transitions`

**What is wrong.** Both mark the subscription's new state durably first (`mark_subscription_*` → status changes so the row leaves the candidate set), then call `emit_scheduler_callback`, whose error is only `error!`-logged. `emit_scheduler_callback` can fail **before** `create_and_forward_webhook` creates the durable `webhook_delivery` row (e.g. `repo.get_app(app_id).await?`). The 3-strike retry only protects deliveries that already exist.

**Why it's a real bug.** The state mutation and the durable callback are not atomic and there is no retry once the row is no longer eligible. The app backend never learns of the cancellation/pause → stale entitlement.

**Failure scenario.** Price step-up deadline expires → `mark_subscription_price_step_up_expired` sets `cancelled` → `emit_scheduler_callback` fails loading app data → error logged → next tick skips the row (no longer expired-candidate) → app never receives `subscription.cancelled`.

**Smallest safe fix.** Create the callback/outbox delivery atomically with the state transition (same tx), or keep a "scheduler-callback-pending" marker cleared only after the delivery row is created, so the work remains retryable. Inline HTTP retry is not sufficient — the durable record is what's lost.

**Regression test.** Yes — when `mark_subscription_*` succeeds but callback creation fails before a delivery row exists, assert the work stays retryable (or a durable delivery/outbox row exists).

---

## 10. Medium — Google Play test notifications ACKed with no durable trace

**File:** [src/webhooks/provider_adapter.rs#L190-L196](file:///c%3A/share/tyde/bridge/src/webhooks/provider_adapter.rs#L190-L196) (`decode_and_normalize`) → [src/webhooks/ingress.rs#L316-L317](file:///c%3A/share/tyde/bridge/src/webhooks/ingress.rs#L316-L317) (`handle_google_play`)

**What is wrong.** A signed `testNotification` returns `Ok(None)` → ingress returns `204` with no `pay.webhook_provider` inbox row and no terminal suppressed state.

**Why it's a real bug.** Violates the "ACK only after a durable provider inbox row and either a delivery work item or terminal suppressed state" invariant. There's no auditable record that the signed event was received and intentionally no-oped.

**Smallest safe fix.** Normalize test notifications into a provider event (e.g. `GOOGLE_PLAY_TEST_NOTIFICATION`), persist to `pay.webhook_provider`, mark suppressed with a reason, then ACK. No app delivery.

**Regression test.** Yes — a test notification produces a durable suppressed provider row before ACK and no `webhook_delivery`.

---

## 12. Medium — Global `purchase_token UNIQUE` breaks app isolation

**File:** [migrations/02_create_subscriptions.sql#L13](file:///c%3A/share/tyde/bridge/migrations/02_create_subscriptions.sql#L13) — `purchase_token TEXT UNIQUE`

**What is wrong.** `purchase_token` is globally unique across the whole table, while all code queries it app-scoped (`WHERE app_id = $1 AND purchase_token = $2`, e.g. [src/db/subscriptions.rs#L956-L958](file:///c%3A/share/tyde/bridge/src/db/subscriptions.rs#L956-L958)).

**Why it's a real bug (partly by-design tension).** The column comment says "One-token-one-owner for fraud prevention," so global uniqueness may be intentional. But it contradicts the RLS/app-isolation model: one app can block another app from inserting the same token value (sandbox reuse, staging/prod split, provider namespace collision), producing a unique-constraint failure across a tenant boundary that RLS otherwise isolates.

**Smallest safe fix.** Decide the intended invariant. If app-scoped: replace with a partial unique index on `(app_id, provider, purchase_token) WHERE purchase_token IS NOT NULL`. If truly global: document it explicitly and confirm all fraud/restore code depends on it. Flagging for a decision rather than a blind change.

**Regression test.** Yes (whichever direction) — two apps with the same token both succeed (app-scoped) OR the second is rejected (global), plus duplicate within `(app_id, provider)` fails.

---

## 13. Low — Poisoned-mutex `.expect()` panics production email path

**File:** [src/services/email.rs#L94-L106](file:///c%3A/share/tyde/bridge/src/services/email.rs#L94-L106) — `start_provider_rate_limit_cooldown`, `provider_rate_limit_cooldown_remaining_seconds`

**What is wrong.** `.lock().expect("email provider cooldown lock poisoned")` on the send path. Violates "No unwrap() in production paths" (expect is the same panic class).

**Failure scenario.** A task panics while holding `provider_rate_limit_cooldown_until`; every subsequent Resend email call then panics instead of returning a `BridgeError`.

**Smallest safe fix.** Return `Result` and map poison to `BridgeError::InternalServerError(...)`, or recover the guard via `into_inner()`.

**Regression test.** Yes — poison a test mutex and assert the helper returns an error, not a panic.

---

## 14. Low — Rate limiter: spoofable client IP + unbounded key growth

**File:** [src/middleware/rate_limit.rs#L129-L149](file:///c%3A/share/tyde/bridge/src/middleware/rate_limit.rs#L129-L149) — `extract_client_ip`; store at [#L49-L120](file:///c%3A/share/tyde/bridge/src/middleware/rate_limit.rs#L49-L120)

**What is wrong.** (a) `X-Forwarded-For` / `X-Real-IP` are trusted before the socket peer, with no trusted-proxy gate (the code comment acknowledges the hazard). (b) The static rate-limit `HashMap` never removes keys, even after their timestamp vectors are pruned empty.

**Why it's a real bug.** If Bridge is ever reachable without a header-stripping proxy in front, an attacker rotates `X-Forwarded-For` to bypass the admin-auth IP guard and public-endpoint limiter, and simultaneously grows the map unboundedly (memory exhaustion). Both are conditional on deployment topology, hence Low.

**Smallest safe fix.** Honor forwarded headers only when the socket peer is a configured trusted proxy; otherwise use `ConnectInfo` peer. After pruning, `remove` empty keys and/or cap total keys.

**Regression test.** Yes — spoofed `X-Forwarded-For` from an untrusted peer maps to the same bucket; stale empty keys are evicted.

