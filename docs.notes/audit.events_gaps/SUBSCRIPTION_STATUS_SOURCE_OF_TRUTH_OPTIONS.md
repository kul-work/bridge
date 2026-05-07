# Subscription Status Source of Truth Options

Date: 2026-05-07

## Context

The current Gap 3 discussion challenges this HiHa behavioral-spec assumption:

> HiHa does NOT call Bridge API to check subscription status.

That assumption should not be treated as settled. It is an architectural choice with real tradeoffs.

The concrete pain point is subscription lifecycle UX: payment failure warnings, price step-up consent, deferred renewal, pause scheduling, revocation details, and reconciliation corrections. Bridge already owns provider lifecycle processing and persists detailed subscription state. HiHa currently has thin callback ingestion and no durable local subscription projection.

## Industry Baseline

In direct Stripe-style integrations, the common pattern is:

- Store subscription identifiers and subscription status in the app database.
- Use provider webhooks to keep that local state updated.
- Query the provider API for checkout confirmation, reconciliation, admin refresh, or suspicious/missing state.
- Avoid querying the provider on every user request unless correctness requirements demand a live check.

That pattern argues for a local projection somewhere. It does not prove that HiHa must own that projection in the Bridge split architecture.

With Bridge, the provider-facing projection already exists in Bridge. HiHa querying Bridge is not the same as HiHa querying Stripe or Google Play directly. Bridge is the internal billing boundary.

## Decision Question

Should HiHa subscription status UX read from:

1. A HiHa-owned local projection populated by Bridge callbacks.
2. Bridge as the authoritative subscription read service.
3. A hybrid model with local coarse entitlement and Bridge-backed detailed status.

The system should explicitly choose one. The current half-state is weak: HiHa toggles local premium from callbacks, but detailed status still depends on a poorly matched Bridge list response.

## Option A: HiHa Local Subscription Projection

HiHa creates a `subscription_cache` or equivalent local projection table. Bridge callbacks populate it. `/api/v1/subscription-status` reads only HiHa DB.

### Arguments For

- HiHa subscription UX remains available when Bridge is down.
- Status reads are fast and do not add a network hop.
- Matches the current HiHa behavioral spec.
- Aligns with the common direct-provider webhook projection pattern.
- Allows app-specific UX fields and messaging to live close to the app.

### Arguments Against

- Duplicates subscription lifecycle state already held by Bridge.
- Requires stale-event suppression, cache update logic, reconciliation handling, and tests in HiHa.
- Creates another place where payment lifecycle semantics can drift.
- Makes Gap 3 a real architectural addition, not a small patch.

### When This Is The Right Choice

Use this if HiHa must render subscription UX during Bridge outages, or if subscription status is hot-path enough that Bridge reads are unacceptable.

## Option B: Bridge Authoritative Read Path

HiHa does not store detailed subscription lifecycle state. It calls Bridge for current subscription status. Bridge owns the detailed read model.

### Arguments For

- Single source of billing lifecycle truth.
- No duplicated Google/Stripe/Creem semantics in HiHa.
- Simpler HiHa implementation.
- Fits Bridge's purpose as central payment processing service.
- Better than asking every app to rebuild its own subscription projection.

### Arguments Against

- HiHa status UX depends on Bridge availability and latency.
- Bridge becomes part of HiHa's runtime read path.
- Needs timeouts, fallback behavior, and potentially load controls.
- Current Bridge list endpoint is not a good UX contract by itself.

### Important Constraint

If this option is chosen, do not keep using the current mismatched list-response parsing. Bridge should expose a purpose-built subscription status snapshot endpoint, or expand the existing API contract deliberately.

Example shape:

```text
GET /api/v1/users/{external_user_id}/subscription-status
```

Response should include app-facing lifecycle fields:

- `is_premium`
- `subscription_id`
- `status`
- `current_period_end`
- `auto_renewing`
- `payment_failure_notification`
- `revoked_at`
- `revocation_reason`
- `google_requires_price_step_up_consent`
- `google_new_price_cents`
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

### When This Is The Right Choice

Use this if correctness and simplicity are more important than status UX independence during Bridge outages.

## Option C: Hybrid

HiHa keeps coarse local entitlement state, such as `users.is_premium`, from callbacks. Detailed lifecycle UX is fetched from Bridge on demand.

### Arguments For

- Keeps hot permission gates local.
- Avoids duplicating all lifecycle details.
- Lets Bridge remain the detailed status authority.
- Better migration path from the current implementation.

### Arguments Against

- Two sources of truth remain, even if their responsibilities are narrower.
- HiHa can show local premium while Bridge detailed status is unavailable.
- Requires clear precedence rules when local premium and Bridge status disagree.

### Suggested Boundary

- HiHa local DB answers: "Can this user access premium app features right now?"
- Bridge answers: "What is the detailed billing/subscription lifecycle state?"

This boundary is defensible, but it must be documented. Without that documentation, future code will drift back into ambiguity.

## Option D: Bridge Read With Short TTL

HiHa calls Bridge but may use a very short in-memory TTL to reduce repeated reads.

### Arguments For

- No durable duplicated state.
- Reduces request bursts against Bridge.
- Easy to remove later.

### Arguments Against

- Still a cache, just less visible.
- Process-local behavior is inconsistent across multiple HiHa instances.
- Does not solve Bridge outage behavior except for a few seconds.

This is a load optimization, not a source-of-truth strategy.

## Strong Recommendation

Do not implement Gap 3 as `subscription_cache` until the source-of-truth decision is made.

If the product wants Bridge as the billing authority, the cleaner fix is:

1. Make Bridge expose a subscription status snapshot API shaped for app UX.
2. Make HiHa `/api/v1/subscription-status` call that API.
3. Keep HiHa callbacks for coarse entitlement, email lookup, idempotency logs, and optional audit.
4. Update HiHa docs to remove the local-cache requirement.

If the product wants HiHa to survive Bridge outages with full subscription UX, then implement the local projection and accept the complexity explicitly.

## Open Questions

- Should detailed subscription UX be available when Bridge is down?
- Is `/api/v1/subscription-status` a hot path or mostly an account/settings path?
- Should app feature gating use live billing state or local entitlement state?
- What is the acceptable staleness window for payment-failure and price-step-up prompts?
- Should Bridge provide one generic status snapshot endpoint for all apps, or should each app compose from existing Bridge APIs?

