# Bridge Code Duplication Audit

This document outlines areas in the Bridge codebase where code is duplicated or follows highly repetitive patterns that could be consolidated.

## 2. Webhook Event Normalization Redundancy

**Locations**: 
- `src/webhooks/processor/normalize.rs`
- `src/services/google_play/provider.rs`

### Mapping Duplication
- `normalize_event_type` in `normalize.rs` maps provider strings to canonical Bridge events.
- `map_subscription_notification_type_to_event` in `provider.rs` maps Google Play notification integers to similar strings.

### Issue
Mapping logic is fragmented. If a new status is added, it must be updated in both the provider implementation and the normalization middleware, leading to potential "ghost" statuses if one is missed.

## 3. Google Play Lifecycle Boilerplate

**Location**: `src/services/google_play/subscription_lifecycle.rs`

### Repetitive Handlers
Handlers like `handle_subscription_revoked`, `handle_subscription_resumed`, and `handle_subscription_cancelled_with_context` share ~80% of their code:
- Extracting `subscription_id` from fields/webhook.
- Calling `repo.apply_subscription_transition`.
- Mapping the result to a `GooglePlayLifecycleOutcome` with boilerplate `callback_event_type` overrides.

## 4. Decentralized Hashing & Anonymization

**Locations**:
- `src/db/users.rs` (`anonymize_user`)
- `src/services/google_play/provider.rs` (`compute_obfuscated_id_hash`)

### Issue
Both modules implement SHA256 hashing for user IDs (one for anonymization, one for Google Play account linking). While they serve different purposes, the underlying cryptographic operation and ID transformation logic are implemented ad-hoc.

## 5. Manual Provider Client Initialization

**Location**: `src/db/users.rs` (`cancel_subscription_at_provider`)

### Issue
This function manually instantiates `CreemClient` and `GooglePlayClient` by parsing raw JSON from the database. This logic bypasses the standard `AppContext` and provider registry used by the rest of the application.

## 6. Port Implementation Overheads

**Location**: `src/ports/impls/subscription.rs`

### Issue
The implementation of `SubscriptionReadRepository` and `SubscriptionWriteRepository` for `db::Database` consists entirely of thin wrappers around `db::subscriptions` functions. While consistent with the Hexagonal architecture, the high granularity of the DB functions (point 1) makes this layer feel like excessive boilerplate.
