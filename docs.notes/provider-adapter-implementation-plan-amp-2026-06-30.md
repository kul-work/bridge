# Provider Adapter Implementation Plan — Amp — 2026-06-30

Goal: make provider-specific webhook semantics explicit at the boundary, so shared lifecycle processing consumes normalized provider input instead of parsing Google Play and Creem payload shapes directly.

This plan follows up on `architectural-review-2026-06-24.md`, item 6. The review is still directionally valid, but one point has changed: Google Play shared-SKU identity is already partly mitigated by purchase-token-first lookup in `src/webhooks/processor.rs`. The remaining architectural issue is that this mitigation still lives inside shared processor logic.

## Target outcome

Bridge should have a provider adapter boundary with this responsibility split:

```text
Provider ingress handler
  └─ verify provider delivery envelope and app token
  └─ ask provider adapter to decode provider event
      └─ persist normalized provider event in webhook log
          └─ shared processor consumes normalized fields
```

The adapter boundary should normalize provider differences before shared lifecycle code sees them:

- provider event id
- occurred-at timestamp
- raw event type
- canonical event type
- lifecycle identity
- product identity
- purchase token / provider transaction id
- provider-specific side-effect requirements, such as Google Play acknowledgement
- raw sanitized payload retained for audit/debugging

## Non-goals

- Do not rewrite the whole webhook processor in one step.
- Do not change database schema unless a later phase proves the existing `webhook_provider` columns cannot carry the normalized data safely.
- Do not remove provider-specific lifecycle behavior such as Google Play price-step-up, pause, defer, and acknowledgement handling. Move boundaries first; behavior change only after parity tests are in place.
- Do not introduce a generic plugin system. Two providers exist today; keep the abstraction small and concrete.

## Phase 1 — Introduce normalized event types without changing behavior

Touch target: 2–3 files.

1. Add a small normalized event struct near the webhook boundary, preferably under `src/webhooks/` rather than `src/services/`:

   ```rust
   struct NormalizedProviderEvent {
       provider: String,
       provider_event_id: String,
       raw_event_type: String,
       canonical_event_type: String,
       occurred_at_ms: Option<i64>,
       subscription_id: Option<String>,
       purchase_token: Option<String>,
       product_id: Option<String>,
       provider_transaction_id: Option<String>,
       payload: serde_json::Value,
   }
   ```

2. Keep it intentionally close to existing `create_webhook_provider(...)` fields so this phase is mostly mechanical.
3. Convert current Google Play and Creem ingress extraction into local builder functions that return `NormalizedProviderEvent`.
4. Keep `src/webhooks/processor/fields.rs` unchanged in this phase.

Verification:

- Existing webhook ingress tests still pass.
- Add only minimal tests for the new builders if existing tests do not already cover Google wrapped Pub/Sub and Creem event id extraction.

## Phase 2 — Make provider adapters explicit

Touch target: 3–5 files.

1. Introduce a small enum or trait for known providers. Prefer an enum first unless dynamic dispatch clearly reduces complexity:

   ```rust
   enum ProviderWebhookAdapter {
       GooglePlay,
       Creem,
   }
   ```

2. Move provider-specific decode/normalize logic from `src/webhooks/ingress.rs` into provider adapter functions:

   - Google Play: Pub/Sub envelope decode, RTDN event type extraction, test notification handling, purchase token/product id extraction.
   - Creem: signature-relevant event id/type extraction, object id/subscription/payment identity extraction.

3. Keep signature verification in ingress for now if moving it would force too many config dependencies through the adapter. The adapter boundary can start at “parse event after signature is valid.”
4. Have both ingress handlers call the adapter and then persist the normalized event.

Verification:

- Existing Google Play ingress tests pass.
- Existing Creem signature/header tests pass.
- Add regression coverage that adapter output matches current persisted values for representative subscription and one-time purchase payloads.

## Phase 3 — Move canonical event normalization out of shared processor

Touch target: 2–4 files.

1. Move provider-specific event normalization from `src/webhooks/processor/normalize.rs` into adapter implementations.
2. Change shared processor code to trust the canonical event type already stored for the webhook, or add a temporary compatibility path if storage still contains only raw event type.
3. Keep status normalization in shared code only when the status vocabulary is genuinely Bridge-owned. Provider raw status mapping belongs in adapters.

Verification:

- `src/webhooks/processor/tests.rs` normalization tests should either move to adapter tests or be rewritten as adapter-output tests.
- Cross-provider canonical event names must remain unchanged.

## Phase 4 — Pull provider field extraction out of shared processor

Touch target: 3–5 files.

1. Replace `extract_webhook_fields(...)` provider switches with fields carried by the normalized provider event.
2. Keep Google Play enrichment that requires live provider API calls separate from raw payload parsing. Name it as enrichment, not parsing.
3. Preserve the existing Google Play purchase-token-first identity rule explicitly in the normalized identity model:

   ```text
   Google subscription lifecycle identity = purchase_token first, product SKU second only for non-user-specific product identity.
   ```

4. Keep Creem subscription id behavior unchanged.

Verification:

- Existing processor tests pass.
- Add or preserve cross-user Google Play tests proving shared SKU cannot select the wrong user/subscription.

## Phase 5 — Provider API boundary cleanup

Touch target: 2–4 files.

1. Replace broad string switches in `src/services/provider_api.rs` with provider-specific functions or adapter methods for supported operations:

   - cancel subscription
   - resume subscription
   - billing portal
   - acknowledge subscription/product
   - fetch subscription status

2. Keep unsupported-operation errors explicit. Do not pretend all providers support the same operations.
3. Avoid a large trait object if the call sites remain simpler with `match ProviderKind`.

Verification:

- Scheduler acknowledgement tests still pass.
- Reconciliation behavior remains unchanged for Google Play and Creem.

## Phase 6 — Remove compatibility branches and document the boundary

Touch target: 2–3 files.

1. Delete temporary compatibility paths from processor once all ingress writes normalized fields consistently.
2. Update `DESIGN.md` with the provider adapter boundary and identity ownership rule.
3. Update `INVARIANTS.md` only if the normalized identity rule becomes a new explicit invariant.

Verification:

- Full targeted webhook/provider test set.
- Manual review checklist:
  - shared processor does not parse Google Play or Creem JSON shapes directly;
  - shared processor does not infer Google Play identity from product SKU;
  - provider-specific acknowledgement requirements are represented before lifecycle processing;
  - callback payloads remain stable.

## Suggested implementation order

Start with Google Play, because it has the highest semantic mismatch: Pub/Sub envelope, purchase tokens, shared SKU, RTDN notification types, and acknowledgement. Then apply the same boundary to Creem.

The safe sequence is:

1. Normalize event shape in ingress with no behavior change.
2. Move parsing into adapter functions.
3. Move canonical event mapping into adapters.
4. Move field extraction into adapters.
5. Clean up provider API switches.
6. Document the final boundary.

## Risk notes

- This is lifecycle-sensitive work. Treat it as payment/provider Tier 2 risk at minimum.
- Google Play shared SKU identity is the trap. Any phase that changes subscription lookup must preserve purchase-token-first behavior.
- Avoid broad refactors while moving the boundary. Each phase should be reviewable by comparing old extracted values to new adapter output.

## Definition of done

- Provider-specific payload parsing is isolated to provider adapter code.
- Shared webhook processor consumes normalized provider event fields.
- Google Play subscription identity remains purchase-token-first.
- Existing webhook, subscription lifecycle, callback, and acknowledgement tests pass.
- `DESIGN.md` or `INVARIANTS.md` documents the final ownership boundary.
