# Bridge Invariants

## Money
- NEVER use f64/f32 for currency. Integer cents only.
- All amounts stored as i64 cents in DB.
- Display formatting happens at API boundary, never in domain logic.

## Status / Lifecycle
- Subscription status MUST be a typed enum, never raw String in domain code.
- Unknown provider statuses → explicit Unknown(String), NEVER silent fallback.
- Status transitions are monotonic: newer timestamp_epoch_ms always wins.
- Webhook replay MUST be idempotent (checked via webhook_log).

## Layer Boundaries
- Handlers: HTTP orchestration only. No business logic. No direct DB writes.
- Services: Provider integration. Translate provider concepts to Bridge domain.
- Application: Business rules. Status transitions. Validation.
- DB: Pure queries. No business decisions.
- Webhooks: Ingress validation → idempotency check → normalize → delegate.

## Webhook Processing
- All webhooks validate provider signature first.
- Idempotency checked via webhook_log BEFORE any mutation.
- Stale events suppressed by comparing timestamp_epoch_ms.
- Callback delivery uses 3-strike retry with exponential backoff.

## Error Handling
- thiserror for typed domain errors.
- anyhow for context propagation.
- No unwrap() in production paths.
- Errors at handler level map to HTTP status codes.
