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
| 2 | High | GP webhook | `packageName` never checked — cross-app RTDN acceptance |
| 3 | High | Reconciliation | Unknown Google status silently mapped to `active` (keeps paid access) |
| 4 | High | Verify path | Unknown Google status silently mapped to `expired` (revokes paid access) |
| 5 | High | Money identity | `provider_transaction_id` falls back to purchase token / RTDN message id |
| 6 | High | RLS | `list_user_subscriptions_to_cancel` bypasses app context → returns zero rows |
| 7 | High | Scheduler | Terminal (cancelled/revoked) subs can be flipped to `paused` |
| 8 | High | Scheduler | Scheduler callbacks lost if enqueue fails after state mutation |
| 9 | Medium | Status norm | `normalize_status` collapses unknown → `None`, missing → `pending` |
| 10 | Medium | GP webhook | Test notifications ACKed with no durable inbox/suppressed row |
| 11 | Medium | Config | Production URL validation allows `http://localhost` |
| 12 | Medium | DB constraint | Global `purchase_token UNIQUE` breaks app isolation |
| 13 | Low | Email | Poisoned mutex `.expect()` panics production email path |
| 14 | Low | Rate limit | Spoofable `X-Forwarded-For` + unbounded key growth |
| 15 | — | Tests | Several tests assert/lock in the buggy behavior above |

---

## 1. High — Pub/Sub JWT accepted without verifying issuer identity (`email` / `email_verified`)

**File:** [src/services/google_play/client.rs#L674-L693](file:///c%3A/share/tyde/bridge/src/services/google_play/client.rs#L674-L693) — `verify_jwt_with_jwk` / `verify_pubsub_signature`

**What is wrong.** The production JWT path verifies the RS256 signature against Google's JWKs, checks `iss = https://accounts.google.com`, `exp`, and (optionally) `aud`. It never verifies that the token's `email` claim equals the configured Pub/Sub push service account, nor that `email_verified == true`. `PubSubClaims` even carries `email: Option<String>` but it is only logged, never compared.

**Why it's a real bug.** Google's authenticated-push contract requires validating **both** `aud` and the service-account `email`/`email_verified`. Any Google-signed OIDC token minted for the same audience — obtainable by other GCP identities — passes. This is a signature/authenticity control on a money-mutating ingress.

**Failure scenario.** A GCP identity (misconfigured internal service, or an attacker able to mint an OIDC token for the Bridge audience) POSTs a crafted RTDN body. Bridge accepts it because it is Google-signed and audience-matched, then processes fake cancellation/revocation/price-change events.

**Smallest safe fix.** Add an expected Pub/Sub service-account email to provider/client config. Extend `PubSubClaims` with `email_verified: Option<bool>`. After signature+audience checks, require `claims.email.as_deref() == Some(expected)` and `claims.email_verified == Some(true)`. Fail closed when verification is enabled but no expected email is configured.

**Regression test.** Yes — accept when all claims match; reject on `email` mismatch; reject when `email_verified` missing/false.

---

## 2. High — Google Play `packageName` never validated (cross-app / cross-tenant acceptance)

**File:** [src/services/google_play/provider.rs#L1291-L1400](file:///c%3A/share/tyde/bridge/src/services/google_play/provider.rs#L1291-L1400) — `verify_and_parse_webhook`

**What is wrong.** The RTDN `DeveloperNotification.packageName` is logged for "audit trail" but never compared to the provider config's expected package before the event is mapped and returned.

**Why it's a real bug.** RTDN `packageName` is the app-identity field. Accepting a notification for a different package under the wrong provider config violates tenant isolation; downstream mutation then proceeds on purchase-token lookup.

**Failure scenario.** App A = `com.tyde.hiha`, App B = `com.tyde.household`. A signed notification for B is routed to A's provider endpoint (topic/subscription misconfig or injection via a valid Pub/Sub identity). Bridge logs the mismatch but still returns a `WebhookEvent`, applying B's lifecycle changes under A's context.

**Smallest safe fix.** Immediately after parsing `DeveloperNotification`, reject when `dev_notification.package_name != self.package_name` with a verification (not parse) error, so retries/config problems surface as auth/routing failures.

**Regression test.** Yes — matching package accepted; mismatch rejected before a `WebhookEvent` is produced (both direct and Pub/Sub-wrapped forms).

---

## 5. High — `provider_transaction_id` polluted with non-economic identifiers

**Current status:** Fixed. Verify purchase no longer falls back to purchase token, Google subscription RTDN enrichment no longer fabricates `google_play_rtdn:*`, Google OTP purchased handling records a payment only when a provider order/transaction id is present, and `subscription.price_changed` no longer falls back to subscription id or webhook id. The related #15 test was updated to assert no RTDN-message-id fallback.

Three call sites store a non-order value as `payments.provider_transaction_id`, violating the money-identity invariant ("`provider_transaction_id` is the provider's economic transaction/order id … purchase tokens must use dedicated token fields").

**5a. Verify commit falls back to purchase token**
[src/application/verify_purchase.rs#L321-L325](file:///c%3A/share/tyde/bridge/src/application/verify_purchase.rs#L321-L325) — `verify_purchase`
```rust
provider_transaction_id: verified
    .provider_transaction_id
    .as_deref()
    .unwrap_or(&payload.purchase_token),
```
The purchase token is passed separately and stored in `provider_purchase_token`, so this fallback only pollutes the economic-id column.

**5b. Subscription RTDN fabricates from Pub/Sub message id**
[src/webhooks/processor.rs#L300-L305](file:///c%3A/share/tyde/bridge/src/webhooks/processor.rs#L300-L305) and [#L463-L465](file:///c%3A/share/tyde/bridge/src/webhooks/processor.rs#L463-L465)
```rust
.unwrap_or_else(|| format!("google_play_rtdn:{}", provider_webhook_id))
...
fields.provider_transaction_id = Some(format!("google_play_rtdn:{}", webhook.provider_webhook_id));
```

**5c. OTP lifecycle falls back to webhook id**
[src/services/google_play/product_lifecycle.rs#L46-L58](file:///c%3A/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L46-L58) — `handle_otp_purchased`
```rust
let txn_id = fields.provider_transaction_id.as_deref()
    .unwrap_or(&webhook.provider_webhook_id);
```

**Why it's a real bug.** The RTDN message id changes per delivery/event; the purchase token is a lifecycle handle. When the real Google `orderId` later arrives, Bridge sees a different id and can create a **second payment row for the same economic transaction**, corrupting dedup, refund lookup, reporting, and audit.

**Failure scenario.** A renewal RTDN arrives before `latest_order_id` is available → payment recorded as `google_play_rtdn:<msgid>`. A later event/reconciliation carries the real order id → duplicate payment.

**Smallest safe fix.** Make `provider_transaction_id` optional through the commit/record path. Write a payment row only when a real economic id exists; otherwise update by `provider_purchase_token` or defer to enrichment/reconciliation. Never substitute token or message id.

**Regression test.** Yes — when the provider yields no order id, assert no payment row is written with `provider_transaction_id` equal to the purchase token or `google_play_rtdn:*`.

---

## 6. High — `list_user_subscriptions_to_cancel` bypasses RLS app context → silently returns no rows

**File:** [src/db/users.rs#L7-L29](file:///c%3A/share/tyde/bridge/src/db/users.rs#L7-L29) — `list_user_subscriptions_to_cancel`

**What is wrong.** This function queries `pay.subscriptions` directly on `pool` without opening a transaction and calling `set_local_app_id` — unlike every other subscription query (which use `begin_app_tx`).

**Why it's a real bug (confirmed).** `pay.subscriptions` has `ENABLE` + `FORCE ROW LEVEL SECURITY` ([migrations/91_fix_rls_current_app_id_cast.sql#L15,L93](file:///c%3A/share/tyde/bridge/migrations/91_fix_rls_current_app_id_cast.sql#L15)) with policy `app_id = pay.current_app_id()`, and the app connects as `bridge_app` ([.env.sample#L2](file:///c%3A/share/tyde/bridge/.env.sample#L2)). With no `bridge.current_app_id` set on the session, `current_app_id()` doesn't match and RLS filters out **all** rows. The `WHERE app_id = $1` predicate does not help because RLS is applied on top.

**Failure scenario.** A user deletion/cancellation flow calls this to find provider subscriptions to cancel. It returns an empty list even when the user has active subscriptions, so Bridge **skips provider-side cancellation** and the user keeps being billed.

**Smallest safe fix.** Mirror the other functions: `begin_app_tx(pool, app_id)` (or `pool.begin()` + `set_local_app_id`), run the query on the tx, commit.

**Regression test.** Yes — under a role/session where RLS applies, insert an active subscription and assert the row is returned.

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

---

## 15. Tests that lock in the bugs above

Current status: partially fixed. The Finding 9 and Finding 5b test lock-ins have been updated; the remaining bullets still pass because the corresponding verification gaps or bugs are unresolved.

Resolved:

- [src/webhooks/processor/tests.rs#L630-L653](file:///c%3A/share/tyde/bridge/src/webhooks/processor/tests.rs#L630-L653) `test_normalize_status` no longer asserts unknown→`None` or missing→`pending`. It now asserts typed status normalization (`Known`, `Unknown`, `Missing`) for Finding 9.
- [src/webhooks/processor/tests.rs#L471-L479](file:///c%3A/share/tyde/bridge/src/webhooks/processor/tests.rs#L471-L479) the Google subscription transaction-id test no longer asserts RTDN message id fallback. It now asserts that missing Google order id yields no synthetic transaction id for Finding 5b.

Still open:

- [tests/creem_webhook_tests.rs](file:///c%3A/share/tyde/bridge/tests/creem_webhook_tests.rs) — `test_creem_signature_header_variations` (currently around L193) and `test_creem_status_normalization` (currently around L246) still assert against local fixture helpers, not the real ingress verifier / adapter, so they'd stay green if production Creem signature verification or status mapping regressed. Convert to integration tests that call the real code paths.
- [src/middleware/rate_limit.rs](file:///c%3A/share/tyde/bridge/src/middleware/rate_limit.rs) `client_ip_prefers_x_forwarded_for` (currently around L564) still reinforces the spoofable behavior in Finding 14.

---

## Needs verification (not proven from code alone)

1. **Callback HMAC does not cover the timestamp.** In [src/webhooks/forwarding.rs#L221-L246](file:///c%3A/share/tyde/bridge/src/webhooks/forwarding.rs#L221-L246) and `#L540-L548`, `X-Pay-Timestamp` is sent but `create_signature` signs only the JSON body. If app backends rely on the timestamp for replay-window checks, they cannot authenticate it. Needs the receiver-side verification contract (app backend or API docs) to confirm whether this is exploitable.

2. **`duplicate_webhook_action(false,false,true) => Ignore`** in [src/webhooks/ingress.rs#L126-L140](file:///c%3A/share/tyde/bridge/src/webhooks/ingress.rs#L126-L140) may suppress a provider retry when a prior delivery row exists but is expired/dead-lettered/stuck. Confirming requires tracing `db/webhooks.rs` delivery retry/dead-letter semantics against the scheduler.

3. **Creem currency extraction completeness.** The amount path uses integer `as_i64()` (invariant-safe), but I did not fully verify currency-field extraction against Creem's payload schema.
