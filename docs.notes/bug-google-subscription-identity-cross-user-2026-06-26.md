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

1. **`update_subscription_status` (reconciliation write-back)** — for `google_play`, key the write strictly on `purchase_token`, NOT `external_user_id`. These are not interchangeable: a user who cancels and resubscribes on the same SKU gets a **new** `purchase_token` but the **same** `(external_user_id, subscription_id)`, so an `external_user_id`-keyed UPDATE would still clobber that user's old row alongside the new one — a within-user version of the same bug. `purchase_token` is the only row-unique key. Scope is contained: `update_subscription_status` has exactly **one** caller (the reconciler at `scheduler.rs:241`), so add a `purchase_token`-keyed mutation and branch by provider in the reconciler. The reconciler row (`sub`) already carries `purchase_token`.
2. **Forward stale-check in `forwarding.rs`** — make the stored-row lookup provider-aware, NOT a blanket swap. For `google_play`, look up by `purchase_token` (already on the payload via `canonical_purchase_token`); for other providers keep `get_subscription_by_sub_id`, because there `subscription_id` is already user-unique and `purchase_token` may be null — a global replacement would neuter stale-suppression for them. This mirrors exactly what `resolve_user` already does. The helper `get_subscription_by_purchase_token` already exists, so this is a one-line lookup swap, no new DB code.

Consider a guardrail: ban generic lifecycle mutations keyed only by `subscription_id` for the `google_play` provider at the repo trait level, so this category cannot regress.

## Done when

- `update_subscription_status` (or its replacement) cannot touch a row for a user other than the one the calling event is about.
- Forward stale-suppression compares against the row matching the *same* purchase token as the incoming event.
- A regression test exists: two rows, same `subscription_id` (`hiha_monthly`), same `app_id`, same `provider`, different `external_user_id` + different `purchase_token`. Run reconciliation / a stale forward against one and assert the other row is untouched. No existing test in `tests/` covers this — that gap is why the bug shipped.
- Bridge checks (Tier 2 — lifecycle + identity) pass on the diff.

## Bug autopsy (why it happened)

The author understood Google's identity model and encoded it in `resolve_user` and the tracing. But that knowledge was not enforced as an invariant at the repository boundary. Two later call sites (`update_subscription_status`, the forward stale-check) reached for the convenient `subscription_id` key without the user dimension, and nothing stopped them. The practical guardrail is exactly that: lifecycle mutations for `google_play` must not accept a `subscription_id`-only key.

-----

## Fixing Strategy

### Goal

Fix Google Play side paths that key subscription lifecycle work by shared SKU
instead of row-unique purchase identity, so reconciliation, forwarding stale
suppression, and retry/resume payload rebuilds cannot cross-contaminate users
sharing the same subscription product.

### Mechanism at a glance

```diagram
╭──────────────────────╮
│ Google Play RTDN/SKU │
│ subscription_id=SKU  │
╰──────────┬───────────╯
           │
           ▼
╭──────────────────────────────╮
│ Resolve row by purchase_token │
│ not SKU for google_play       │
╰───────┬───────────┬──────────╯
        │           │
        ▼           ▼
╭──────────────╮  ╭────────────────╮
│ Reconciler   │  │ Forward stale   │
│ update row id│  │ check same token│
╰──────────────╯  ╰────────────────╯
        │           │
        ╰─────┬─────╯
              ▼
╭──────────────────────────────╮
│ Callback/retry payload uses   │
│ same subscription row/token   │
╰──────────────────────────────╯
```

### Already-decided — not re-litigating

- Do not re-key the whole Google integration; the live webhook mutation path
  already resolves Google users by purchase token.
- Do not replace all provider stale checks with purchase-token lookup; non-Google
  providers can keep subscription-id keying where that id is user-unique.
- Do not key Google reconciliation by `external_user_id + subscription_id`; the
  same user can resubscribe to the same SKU with a new purchase token.
- For reconciliation, row `id` is the write key. Purchase token is the Google
  identity concept, but the reconciler already has a concrete DB row, so row id
  is the narrowest safe mutation target.
- For Google stale checks and retry/rebuild, a missing or unmatched purchase
  token must never fall back to SKU-only row lookup. Skip stale suppression and
  forward; rebuild from webhook fields without borrowing another row's state.
- Keep the existing purchase-token lookup helper. Do not add a provider-specific
  token helper/refactor just for this fix; `purchase_token` is already unique in
  `pay.subscriptions`.
- Minimal guardrail first; repo-trait-level bans can be follow-up unless review
  finds the minimal guardrail insufficient.

**Problem context:** Two Google Play `Subscription` rows can share the same
`app_id/provider/subscription_id` but differ in `external_user_id`,
`purchase_token`, or `last_event_time`. Any logic keyed on `subscription_id`
alone risks updating or evaluating the wrong row.

**Google docs basis:** `docs/google/GOOGLE_PLAY_BILLING_TESTPLAN.md` says
purchase verification registers `(user_id, purchase_token, subscription_id)` and
subsequent webhooks use `purchase_token` to look up the user. The same test plan
and `docs/google/GOOGLE_PLAY_SUBSCRIPTION_LIFECYCLE-v1.1.md` also document that
resubscribe flows issue a **new** purchase token on the same subscription SKU,
and webhooks that arrive before purchase-token registration are safe to ignore
or process without DB mutation until verification registers the token.

### Core changes

1. **Reconciliation**
   Update by row `id` from the reconciled `Subscription`, not by
   `subscription_id`. The reconciler already iterates concrete DB rows, so the
   row's own primary key is the safe identifier. Replace the scheduler-facing
   `update_subscription_status(app_id, subscription_id, ...)` path with an
   id-keyed mutation such as `update_reconciled_subscription_status(app_id, id,
   ...)`. Preserve the current app scope and high-water stale guard:
   `WHERE app_id = ? AND id = ? AND last_event_time < event_time_ms`.

2. **Forward stale check**
   For `google_play`, compare staleness using `payload.purchase_token` (i.e.,
   `canonical_purchase_token`) instead of `subscription_id`. Use the existing
   `get_subscription_by_purchase_token(app_id, token)` helper; no new provider
   lookup is needed for this fix because `purchase_token` is unique. If no token
   is present or the token lookup misses, skip stale suppression and forward the
   event rather than suppressing it based on a shared SKU. This fail-open rule
   applies only to token absence/miss; DB errors should still fail normally.

3. **Retry/resume canonical payload rebuild**
   For `google_play`, rebuild from the subscription row found by purchase token.
   If the token is missing or unmatched, do **not** fall back to
   `subscription_id`-only lookup. Build from the webhook fields/resolved user and
   avoid row-derived status/token/period fields rather than borrowing another
   user's same-SKU row.

4. **Guardrail**
   Keep the guardrail minimal in this fix: replace the scheduler's ambiguous
   subscription-id update path with the id-keyed reconciler mutation, and make
   the two Google read paths explicitly purchase-token keyed. Do not do a broad
   repo-trait-level ban/refactor in this phase unless implementation proves the
   surgical change cannot be made safely.

5. **Regression test**
   Two Google rows, same `app_id/provider/subscription_id`, different
   `external_user_id`/`purchase_token`/`last_event_time`. Cover all three paths:
   reconciliation updates only the intended row; forwarding stale suppression
   compares against the row with the same purchase token; retry/rebuild payloads
   use the same-token row and never borrow state from a same-SKU row. Also cover
   the best practical token-missing/miss case: Google should not suppress or
   rebuild from a SKU-only wrong row when the purchase token is absent or not
   found.

### Execution

- **Priority order:**
  1. Reconciliation fix
  2. Forwarding stale-suppression fix
  3. Retry/resume canonical payload rebuild fix
  4. Two-user/same-SKU regression tests covering all three paths
- **Validation:** Bridge Tier-2 checks + `cargo check` + `cargo clippy` + targeted
  `cargo test` regression coverage for all three paths.
