# Bridge Code Duplication Audit

This document outlines areas in the Bridge codebase where code is duplicated or follows highly repetitive patterns that could be consolidated.

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
