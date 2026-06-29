# Bridge Invariants

## Money
- NEVER use f64/f32 for currency. Integer cents only.
- All amounts stored as i64 cents in DB.
- Display formatting happens at API boundary, never in domain logic.
- `payments.provider_transaction_id` is the provider's economic transaction/order id. Google Play purchase tokens are lifecycle/API handles and must use dedicated token fields such as `payments.provider_purchase_token` or `subscriptions.purchase_token`.

## Status / Lifecycle
- Subscription status MUST be a typed enum, never raw String in domain code.
- Unknown provider statuses → explicit Unknown(String), NEVER silent fallback.
- Status transitions are monotonic: newer timestamp_epoch_ms always wins.
- Webhook replay MUST be idempotent (checked via webhook_provider).

## Layer Boundaries
- Handlers: HTTP orchestration only. No business logic. No direct DB writes.
- Services: Provider integration. Translate provider concepts to Bridge domain.
- Application: Business rules. Status transitions. Validation.
- DB: Pure queries. No business decisions.
- Webhooks: Ingress validation → idempotency check → normalize → delegate.

## Webhook Processing
- All webhooks validate provider signature first.
- Idempotency checked via webhook_provider BEFORE any mutation.
- Stale events suppressed by comparing timestamp_epoch_ms.
- Provider ACK is sent only after Bridge has a durable provider inbox row and either a durable `webhook_delivery` work item or a terminal suppressed state. Provider enrichment, user resolution, subscription mutation, and app forwarding run after that enqueue point.
- Callback delivery uses 3-strike retry with exponential backoff.

## Observability / PII
- Logs and traces stay conservative by default; use hashed diagnostics when sensitive identifiers are needed for correlation.
- Never log raw purchase tokens, API keys, HMAC secrets or signatures, authorization headers, or raw provider callback bodies.
- Emails appear in normal logs only when intentionally needed, preferably scrubbed.
- Admin notification emails may include operational context when required.

## Error Handling
- thiserror for typed domain errors.
- anyhow for context propagation.
- No unwrap() in production paths.
- Errors at handler level map to HTTP status codes.
