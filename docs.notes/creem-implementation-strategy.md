# Creem Implementation Strategy For Bridge

## Purpose

This note captures the current state of Creem in Bridge, how the Bridge design docs should be used, and the recommended implementation strategy to bring Creem to parity with the old working monolith behavior while staying aligned with Bridge architecture.

## Summary

Creem is not fully implemented in Bridge today.

- Bridge does have live Creem code paths for checkout, webhook ingress, provider API actions, and some event normalization.
- The archived [`src/services/creem.rs`](file:///c:/share/tyde/bridge/src/services/creem.rs) is orphaned and not part of runtime wiring.
- The active Bridge Creem path has drifted from the old monolith behavior that was validated by the CBI tests in `c:\share\hiha\tests\cbi`.
- The biggest gaps are in webhook signature/header handling and payload field mapping.

## How To Use The Existing Docs

The existing docs are useful as the Bridge target contract, but they do not fully describe the raw Creem provider payloads.

### Helpful Design Guidance

[`docs.notes/pay-tydecode-architecture.md`](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-architecture.md) is useful because it defines the intended Bridge responsibilities:

- Bridge owns provider webhook ingress and signature verification.
- Bridge owns idempotent webhook processing via `webhook_provider`.
- Bridge owns normalized callbacks to apps.
- Creem is intended to be a supported provider inside Bridge's abstraction model.
- Creem provider configuration belongs in `provider_configs.config`.

Relevant sections:

- [`pay-tydecode-architecture.md` overview and responsibilities](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-architecture.md#L12-L12)
- [`pay-tydecode-architecture.md` webhook/provider responsibilities](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-architecture.md#L35-L39)
- [`pay-tydecode-architecture.md` Creem provider config example](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-architecture.md#L146-L154)
- [`pay-tydecode-architecture.md` webhook_provider schema and idempotency boundary](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-architecture.md#L320-L344)

[`docs.notes/pay-tydecode-api-contract.md`](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-api-contract.md) is useful because it defines the expected external Bridge API behavior:

- Creem is a web checkout provider.
- `verify-purchase` is a mobile-only flow.
- Bridge must expose normalized callback payloads to apps.
- Webhook ingress must verify provider-specific cryptographic signatures.

Relevant sections:

- [`pay-tydecode-api-contract.md` checkout contract](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-api-contract.md#L70-L130)
- [`pay-tydecode-api-contract.md` verify-purchase contract](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-api-contract.md#L142-L165)
- [`pay-tydecode-api-contract.md` subscription management and portal contract](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-api-contract.md#L263-L400)
- [`pay-tydecode-api-contract.md` provider webhook ingress and callback contract](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-api-contract.md#L650-L766)

### What The Docs Do Not Cover

The docs do not fully specify:

- the exact raw Creem signature header name
- the exact raw Creem webhook payload field shapes
- the special handling required for `checkout.completed`
- the provider-specific fallbacks needed to resolve `external_user_id`, `product_id`, `purchase_token`, and `amount_cents`

For those details, the old monolith implementation and CBI tests remain the best source of truth.

## Current State In Bridge

### What Is Live

Bridge currently contains active Creem logic in these areas:

- [`src/application/checkout.rs`](file:///c:/share/tyde/bridge/src/application/checkout.rs) for checkout creation
- [`src/webhooks/mod.rs`](file:///c:/share/tyde/bridge/src/webhooks/mod.rs) and [`src/webhooks/ingress.rs`](file:///c:/share/tyde/bridge/src/webhooks/ingress.rs) for Creem webhook routing and ingress
- [`src/webhooks/processor.rs`](file:///c:/share/tyde/bridge/src/webhooks/processor.rs) for event normalization and canonical callback building
- [`src/services/provider_api.rs`](file:///c:/share/tyde/bridge/src/services/provider_api.rs) for cancel, resume, billing portal, and status fetch
- [`src/db/users.rs`](file:///c:/share/tyde/bridge/src/db/users.rs) for provider-side cancel during user cleanup flows

### What Is Dead

[`src/services/creem.rs`](file:///c:/share/tyde/bridge/src/services/creem.rs) is an archived copy of the old trait-based provider shape and is not wired into Bridge runtime.

[`src/services/mod.rs`](file:///c:/share/tyde/bridge/src/services/mod.rs) does not export it, and no runtime code instantiates `CreemProvider`.

## Main Gaps Found During Investigation

### 1. Signature Header Drift

Bridge ingress currently expects:

- `Webhook-Signature`
- `x-signature`

See [`src/webhooks/ingress.rs`](file:///c:/share/tyde/bridge/src/webhooks/ingress.rs#L18-L18).

But the archived provider and old CBI tests use `creem-signature`:

- [`src/services/creem.rs` signature header name](file:///c:/share/tyde/bridge/src/services/creem.rs#L539-L548)
- [`c:\share\hiha\tests\cbi\test-whk-01.sh`](file:///c:/share/hiha/tests/cbi/test-whk-01.sh#L113-L119)

This is a real compatibility gap.

### 2. Payload Mapping Drift

Bridge's active webhook field extraction expects a payload shape that does not match the monolith-tested Creem payloads.

Current Bridge field extraction is in:

- [`src/webhooks/processor.rs`](file:///c:/share/tyde/bridge/src/webhooks/processor.rs#L237-L262)

The old working monolith logic handles richer field fallbacks and special cases in:

- [`c:\share\hiha\src\services\creem.rs`](file:///c:/share/hiha/src/services/creem.rs#L237-L356)

Important fields that need parity:

- `object.id`
- `object.product_id`
- `object.current_period_end_date`
- `object.last_transaction.amount`
- `object.metadata.user_id`
- `object.checkout.metadata.user_id`
- `checkout.completed` recurring vs one-time behavior

### 3. Bridge Contract Must Stay Normalized

Even when provider payloads are inconsistent, Bridge must still emit the normalized callback contract described in [`pay-tydecode-api-contract.md`](file:///c:/share/tyde/bridge/docs.notes/pay-tydecode-api-contract.md#L683-L766).

The app callback needs stable fields like:

- `event_type`
- `external_user_id`
- `provider`
- `subscription_id`
- `product_id`
- `status`
- `amount_cents`
- `purchase_token`
- `timestamp`
- `timestamp_epoch_ms`

## Recommended Strategy

Implement Creem inside Bridge's current runtime architecture, then remove the archived duplicate.

Do not revive the old monolith provider factory pattern as the primary runtime path.

### Phase 1 - Webhook Ingress Parity

Goal: make Creem webhooks enter Bridge correctly and safely.

Work:

- Update Creem ingress signature extraction to support the real Creem header contract, including `creem-signature`.
- Keep provider signature verification at ingress, before mutation.
- Preserve the `webhook_provider` idempotent boundary.
- Make sure valid Creem events reach processing instead of failing at the door.

Likely files:

- [`src/webhooks/ingress.rs`](file:///c:/share/tyde/bridge/src/webhooks/ingress.rs)
- possibly related ingress tests in the same file

Why first:

- If ingress rejects real Creem events, the rest of the pipeline does not matter.

### Phase 2 - Webhook Payload Mapping Parity

Goal: teach Bridge's active webhook processor the real Creem payload variants that were already proven in the monolith.

Work:

- Port the necessary field extraction and normalization behavior from the old monolith Creem parser into Bridge's live processor flow.
- Handle `subscription.active`, `subscription.trialing`, `refund.created`, and both flavors of `checkout.completed`.
- Preserve the Bridge canonical callback model while broadening raw payload compatibility.
- Ensure `external_user_id` resolution checks all valid Creem metadata locations before falling into orphan suppression.

Likely files:

- [`src/webhooks/processor.rs`](file:///c:/share/tyde/bridge/src/webhooks/processor.rs)

Why second:

- This is the core of whether Bridge actually understands real Creem events.

### Phase 3 - Checkout And Provider API Parity

Goal: ensure Bridge-originated Creem sessions and subscription actions match the intended Bridge contract.

Work:

- Validate checkout metadata sent from [`src/application/checkout.rs`](file:///c:/share/tyde/bridge/src/application/checkout.rs) is sufficient for later webhook resolution.
- Confirm cancel, resume, portal, and status-fetch behavior in [`src/services/provider_api.rs`](file:///c:/share/tyde/bridge/src/services/provider_api.rs).
- Keep `verify-purchase` limited to mobile-store semantics per the API contract rather than over-expanding Creem there.

Likely files:

- [`src/application/checkout.rs`](file:///c:/share/tyde/bridge/src/application/checkout.rs)
- [`src/services/provider_api.rs`](file:///c:/share/tyde/bridge/src/services/provider_api.rs)
- possibly [`src/application/verify_purchase_provider.rs`](file:///c:/share/tyde/bridge/src/application/verify_purchase_provider.rs) only if a small cleanup is needed

Why third:

- Webhook correctness is the biggest missing production behavior. Provider action parity matters next.

### Phase 4 - Regression Tests Based On Old CBI Scenarios

Goal: lock in parity so Creem does not drift again.

High-value cases to port into Bridge tests:

- valid `subscription.active`
- recurring `checkout.completed`
- one-time `checkout.completed`
- `refund.created`
- invalid signature rejection

Preferred approach:

- add a small number of Rust tests around ingress and processor normalization
- favor high-leverage end-to-end-ish tests over many tiny helper tests

### Phase 5 - Remove The Archived Split-Brain Code

Goal: restore a single source of truth.

Once the active Bridge runtime path is complete and covered by tests:

- delete or fully retire [`src/services/creem.rs`](file:///c:/share/tyde/bridge/src/services/creem.rs)

Leaving both paths around is what created the current confusion and drift.

## Practical Rule Of Implementation

Use three inputs together:

- Bridge design docs as the architectural target
- Bridge API contract as the external behavior target
- old monolith Creem implementation and CBI tests as the raw Creem compatibility oracle

That combination gives the safest port strategy:

- stay true to Bridge design
- keep the public contract clean
- preserve the provider-specific behavior that was already proven to work

## Suggested First Execution Slice

If implementation starts next, the best first slice is:

1. fix Creem signature header handling in ingress
2. fix active payload mapping for `subscription.active` and `checkout.completed`
3. add one regression test proving ingress + normalization + user resolution works

That slice is small, high impact, and directly addresses the biggest evidence of incomplete Creem support in Bridge.
