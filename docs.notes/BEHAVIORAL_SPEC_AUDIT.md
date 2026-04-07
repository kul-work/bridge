# Behavioral Spec Alignment Audit

Date: 2026-04-06

## Scope

Compared [docs.notes/BEHAVIORAL_SPEC.md](./BEHAVIORAL_SPEC.md) against the current Bridge implementation in `src/` and the supporting schema in `migrations/`.

Status labels used below:

- `Partial`: the flow exists, but one or more contract details differ.
- `Diverged`: the implementation intentionally or materially behaves differently.
- `Missing`: the spec describes behavior that is not implemented.

The spec itself is marked `Proposal / Under Review`, so some items below are probably stale-spec issues rather than code bugs. Where that seems likely, it is called out directly.

## Executive Summary

High-confidence mismatches worth attention first:

1. Provider credential storage does not match the spec. Code reads provider config from `pay.provider_configs`, while the spec repeatedly describes app-scoped credentials loaded from `apps` with an extra decryption step.
2. The documented per-endpoint default rate limits are not what the code actually enforces. In practice, the app-wide default of `120/min` wins unless JSON overrides are configured.
3. Unauthenticated IP rate limiting ignores `X-Forwarded-For` and `X-Real-IP`, so deployed behavior behind a proxy will drift from the spec.
4. Webhook ingress is looser than the spec for invalid tokens and missing provider event IDs.
6. `verify-purchase` does not currently record provider amounts even though the spec treats `amount_cents` as part of the verification result.
7. Agent token creation semantics differ materially from the documented contract.
8. GDPR anonymization and data export behavior diverge from the spec in user-visible ways.

## Detailed Findings

| Priority | Spec Section(s) | Status | Finding | Code Evidence |
|---|---|---|---|---|---|
|#20| P2 | 1 | Partial | Startup/background workers largely match the spec, but the implementation also starts a webhook retry worker that the startup section does not mention explicitly. This is not harmful, but it means the spec is incomplete about active background jobs. | `src/main.rs:82-89` |
|#21| P2 | 4 | Partial | Checkout behavior is mostly aligned, but the implementation has an extra alias route (`/api/v1/payment/checkout`) that the spec does not document. | `src/main.rs:97-99` |


## Likely Stale-Spec Areas

These look more like documentation drift than broken code, but they should still be resolved so the spec can be used safely as an implementation contract:

- The spec still says provider credentials come from the `apps` table, while the implementation and schema have clearly standardized on `pay.provider_configs`.
- The spec still describes an extra decryption step for provider credentials, but the current implementation reads provider config directly from `pay.provider_configs`.
- Some response shapes in the spec (`register_purchase`, `resume_subscription`, `agent/token`, `agent/charge`) no longer match the actual API.
