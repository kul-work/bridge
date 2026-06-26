# BUG — Google subscription identity can corrupt entitlement across users

- **Severity:** Critical (money + entitlement, cross-user)
- **Status:** Open
- **Discovered:** 2026-06-26, during architectural review (`docs.notes/architectural-review-2026-06-24.md`, section 1)
- **Window:** Found **before** PROD/LIVE. No real customers affected yet. This ticket exists because this is the most critical bug found in the platform so far and must not get lost.
- **Related review item:** "Critical — Google subscription identity is not safe enough"

## Big picture

The live webhook path is built correctly — it keys user identity on Google's **purchase token**, not on `subscription_id`. There is even a comment in the code saying so (`src/webhooks/processor.rs:554`):

> Google Play subscription_id is a shared product id, so prefer the purchase token which uniquely identifies the user's subscription.

So the central concept was understood. What was missed: **two side-paths silently reverted to `subscription_id`-only keying** and drop the user dimension. With one license tester at a time they never fired. With two real customers on the same product (`hiha_monthly`) in different lifecycle states, they corrupt entitlement — silently, no error, looks fine in logs.

This is not "the whole Google integration is wrong." It is "two call sites need `external_user_id` (or purchase_token) threaded in." Scare is real, scope is small.

## What's actually true about Google's identity

- Google's `subscriptionNotification.subscriptionId` is the **product SKU** (e.g. `hiha_monthly`), shared by every user on that product. It is NOT a per-user lifecycle id.
- Bridge's own tracing aliases it to `product_id` (`src/webhooks/processor.rs:998-1000`).
- The per-user lifecycle id is the **purchase token** (unique per purchase), already stored in `subscriptions.purchase_token` with a `UNIQUE` constraint.
- The live webhook path resolves the user from the purchase token (`resolve_user`, hardcoded to skip sub_id lookup for Google, `processor.rs:556-572`) and every downstream mutation carries `external_user_id` alongside `subscription_id`, so the compound key `(app_id, external_user_id, subscription_id, provider)` disambiguates. **Cross-user contamination is impossible on the live webhook path.** That is why the 4-6 manual platform sessions against real Google servers worked.

## The two unsafe paths

### 1. Reconciliation write-back (the bad one)

- `src/webhooks/scheduler.rs:241` calls `update_subscription_status(repo, app_id, &sub.subscription_id, ...)`.
- The SQL keys on `app_id + subscription_id` only — no `external_user_id`, no `purchase_token` (`src/db/subscriptions.rs:1419`).
- The reconciler loop iterates per-row (so per-user) and queries Google correctly by purchase token, but writes the answer back by `subscription_id` alone.

**Failure mode:** Two customers active on `hiha_monthly`. Reconciler fixes user A → UPDATE sets **both** rows to A's status. Then fixes user B → UPDATE sets both rows to B's status. Last writer wins, for both rows.

- A paying active customer gets flipped to cancelled → loses entitlement.
- A cancelled customer gets resurrected to active → free access.

### 2. Forward stale-suppression (the subtler one)

- `src/webhooks/forwarding.rs:54-90` checks "is this event older than what we already stored?" before forwarding to HiHa.
- The stored-row lookup is `get_subscription_by_sub_id(app_id, subscription_id)` (`forwarding.rs:56`) — no user, no purchase token.
- User A's incoming event can be measured against user B's newer `last_event_time` and get silently suppressed.

**Failure mode:** A cancellation, expiry, or refund RTDN for the wrong user disappears into the void. HiHa never hears about it → entitlement drifts indefinitely.

## Why manual testing never caught it

- **Reconciliation clobber:** needs two license testers on `hiha_monthly` in *different* states when the reconciler runs. License testers are a handful, usually one logged in at a time, usually all in the same state. The double-UPDATE wrote the same value → no visible drift.
- **Forward stale-suppression:** needs out-of-order delivery *across* two users sharing a product where the wrong user's row has the newer timestamp. Sequential manual testing never lines that up.

## Fix scope (surgical)

Do NOT re-key the whole Google integration — the live path is already correct. Only the two call sites that dropped the user dimension:

1. **`update_subscription_status`** — add `external_user_id` (or `purchase_token`) to the WHERE clause, or replace its callers with a row-id / purchase-token-keyed mutation. Reconciliation must write to one row, not all rows sharing a `subscription_id`.
2. **Forward stale-check in `forwarding.rs`** — look up the stored row by `purchase_token` (already available on the payload via `canonical_purchase_token`) instead of `get_subscription_by_sub_id`. Same pattern `resolve_user` already uses for Google.

Consider a guardrail: ban generic lifecycle mutations keyed only by `subscription_id` for the `google_play` provider at the repo trait level, so this category cannot regress.

## Done when

- `update_subscription_status` (or its replacement) cannot touch a row for a user other than the one the calling event is about.
- Forward stale-suppression compares against the row matching the *same* purchase token as the incoming event.
- A regression test exists: two rows, same `subscription_id` (`hiha_monthly`), same `app_id`, same `provider`, different `external_user_id` + different `purchase_token`. Run reconciliation / a stale forward against one and assert the other row is untouched. No existing test in `tests/` covers this — that gap is why the bug shipped.
- Bridge checks (Tier 2 — lifecycle + identity) pass on the diff.

## Bug autopsy (why it happened)

The author understood Google's identity model and encoded it in `resolve_user` and the tracing. But that knowledge was not enforced as an invariant at the repository boundary. Two later call sites (`update_subscription_status`, the forward stale-check) reached for the convenient `subscription_id` key without the user dimension, and nothing stopped them. The practical guardrail is exactly that: lifecycle mutations for `google_play` must not accept a `subscription_id`-only key.
