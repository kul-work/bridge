# Bridge Architectural Review — 2026-06-24

Scope: high-impact architectural risks only. This intentionally excludes small fixes, tracing/logging nits, and PII-only observations unless they expose a larger design problem.

## Executive Summary

Bridge has strong stated principles around idempotency, stale-event suppression, provider normalization, and callback delivery. The biggest risks are not local Rust style issues; they are system-level correctness risks around identity, durable processing, worker coordination, and state ownership.

The most important theme: Bridge currently looks like a payment event system, but several critical paths still behave like best-effort async request handling. That is risky for a service that must survive provider retries, crashes, horizontal scaling, and multiple users sharing the same provider product identifiers.

---

## 6. High — Provider abstraction exists in name, but provider semantics leak everywhere

Bridge claims provider abstraction, but Google/Creem logic is spread through ingress, processor, scheduler, and provider API switches.

### Evidence

- Provider-specific webhook handlers live directly in ingress: `src/webhooks/mod.rs` and `src/webhooks/ingress.rs`.
- Processor contains Google-specific enrichment: `src/webhooks/processor.rs`.
- Provider API switches on provider strings: `src/services/provider_api.rs`.

### Why it matters

Provider-specific identity, timestamp, and status semantics reach the shared lifecycle layer. That is likely why Google product identity leaks into subscription lifecycle identity.

### Recommended architectural direction

Define a provider adapter boundary that emits normalized domain input:

```text
ProviderWebhookAdapter
  verify_signature(...)
  parse_event(...) -> ProviderEvent {
    provider_event_id,
    occurred_at,
    lifecycle_identity,
    product_id,
    raw_status,
    canonical_event,
    side_effect_requirements
  }
```

The application layer should consume `ProviderEvent`, not parse Google/Creem JSON or infer provider identity rules itself.
