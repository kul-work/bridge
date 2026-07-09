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
| 10 | Medium | GP webhook | Test notifications ACKed with no durable inbox/suppressed row |
| 12 | Medium | DB constraint | Global `purchase_token UNIQUE` breaks app isolation |


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