# Bridge - Architecture Decisions

Why key patterns exist. Use as RAG material and integration reference.

## Webhook Processing

**Decision**: Idempotent ingress logging before state mutation

**Why**: Prevents duplication from provider retries. A webhook may arrive 2-3 times; we log the arrival first, then check if already processed.

**Where**: `src/webhooks/` - webhook_log table checks before subscription state changes

**Constraint**: Every webhook handler must validate `webhook_log` before touching `pay.subscriptions` or related state

---

## Error Handling Strategy

**Decision**: Use `thiserror` for domain errors, `anyhow` for context/wrapping

**Why**: 
- Domain errors (PaymentError, ValidationError) are predictable and handled
- Operational errors (DB connection, timeout) are wrapped with `anyhow` for debugging

**Where**: All handler responses return `Result<T, DomainError>`. Operational errors are caught at Axum middleware level.

**Pattern**:
```rust
pub enum PaymentError {
    InvalidProvider(String),
    WebhookValidationFailed,
    SubscriptionNotFound,
}

// vs anyhow for unexpected:
let db_result = sqlx_query().execute(&pool).await
    .context("Failed to update subscription in DB")?;
```

---

## Database Schema Organization

**Decision**: Separate tables by domain (subscriptions, webhooks, provider_configs) in `pay` schema

**Why**: Cleaner queries, easier migrations, clear ownership of what each module touches

**Where**: `migrations/` - each domain gets its own migration file prefix

**Constraint**: Cross-domain queries should be minimal. If you need data from multiple domains, it's a sign the schema needs revision.

---

## Provider Integration Pattern

**Decision**: Provider modules (creem, google_play, etc.) implement a shared interface

**Why**: Normalizes state across different provider APIs (each has different terminology for "active", "paused", "cancelled")

**Where**: `src/services/` - each provider module exports a `Provider` trait implementation

**Constraint**: New providers must map their states to Bridge's canonical states (active, paused, cancelled, expired)

---

## Signature Validation

**Decision**: Double-ended HMAC validation (provider → Bridge AND Bridge → provider webhooks)

**Why**: 
- Provider signatures prove webhook came from them
- Our signatures on return webhooks prove we're legitimate

**Where**: 
- Inbound: `src/webhooks/` validates provider's HMAC header
- Outbound: `src/handlers/` signs webhook delivery responses

**Constraint**: Never process a webhook without validating signature first. Never send to external systems without signing.

---

## Timestamp Handling

**Decision**: Store all timestamps as epoch milliseconds (`timestamp_epoch_ms`), no timezone handling in Bridge

**Why**: 
- Avoids timezone complexity
- Providers send epoch values anyway
- Reconciliation compares raw numbers (no conversion errors)

**Where**: All tables have `timestamp_epoch_ms` field. Business logic compares via high-water mark.

**Constraint**: When comparing old vs new state, use high-water point (newer timestamp wins). Prevents stale events overwriting fresh ones.

---

## Minimal PII Storage

**Decision**: Store opaque `external_user_id` mappings only. Avoid storing names, emails in Bridge DB.

**Why**: 
- Bridge is a payment service, not a user service
- Reduces data breach scope
- Client apps own user data

**Where**: `pay.apps` and related tables use external IDs only

**Constraint**: Don't add user name, email, or personal data to any Bridge tables. Use external ID as join key.

---

## Testing Pattern

**Decision**: Use `#[tokio::test]` for async tests, SQLx for test DB setup

**Why**: Native async support, database tests use real schema (not mocks)

**Where**: `tests/` directory, individual test files per module

**Constraint**: Tests should use real DB when possible. Mock only external services (payment providers, HTTP calls).

---

## Configuration

**Decision**: Environment variables only, no config files

**Why**: Enables easy deployment across environments (local, staging, prod)

**Where**: `src/config.rs` reads from env at startup

**Constraint**: All secrets must be env vars. No hardcoding, no config files in repo.
