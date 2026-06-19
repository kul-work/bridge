# Bridge Deserialization Security Audit

**Date:** 2026-06-19
**Scope:** All deserialization paths in the Bridge codebase — webhook ingress, request bodies, provider API responses, JWT tokens, cursor params, DB rows.
**Status:** Findings #1, #2, #3, #4 implemented. #5, #6 need no action.

---

## Overall Assessment

Bridge's deserialization surface is **moderately secure**. In the normal production configuration, external webhook bodies are verified before JSON parsing and request headers cannot disable verification outside mock mode. That boundary still depends on provider DB config (`verify_webhook_signature`), and Google Pub/Sub audience verification is optional (`GOOGLE_VERIFY_AUDIENCE=false` by default), so treat it as a configured security boundary rather than an unconditional guarantee. No unsafe patterns (`untagged` enums, custom `deserialize_with`, polymorphic deserialization) exist. The main gaps are defense-in-depth issues, not directly exploitable production vulnerabilities.

---

## Findings by Severity

### MEDIUM — `X-Test-Price-Cents` header injection runs unconditionally

**Location:** `src/webhooks/ingress.rs:260-264`

```rust
// Inject test price override into payload for mock-mode enrichment
if let Some(price_str) = headers.get("X-Test-Price-Cents").and_then(|h| h.to_str().ok()) {
    if let Ok(cents) = price_str.parse::<i64>() {
        google_play_event["_test_price_cents"] = serde_json::Value::Number(cents.into());
    }
}
```

The header value is injected into the payload `Value` **without checking if mock mode is enabled**. The downstream consumer (`src/webhooks/processor.rs:440`) is correctly guarded by `mock_external_apis_enabled()`, so the price override only takes effect in mock mode. However:

- The injected `_test_price_cents` field is **persisted to the DB** (`webhook_providers.payload`) regardless of mode — it pollutes production webhook logs/payloads with test data.
- If signature verification is disabled (test env, `MOCK_EXTERNAL_APIS=true`, or misconfigured DB `verify_webhook_signature=false`), any caller can inject arbitrary price values via this header. `MOCK_EXTERNAL_APIS=true` is blocked at production startup, but DB config can still disable provider verification.
- In production with Google JWT verification on, an attacker cannot reach this code without a valid Google-signed Pub/Sub JWT, so it's not directly exploitable — but it violates defense-in-depth.

**Recommendation:** Guard the injection with `if crate::config::mock_external_apis_enabled()` to match the downstream consumer's guard.

---

### LOW-MEDIUM — No `deny_unknown_fields` on deserialized request/query structs

**Scope:** Active code and tests — no struct uses `#[serde(deny_unknown_fields)]`.

Affected user-facing request/query/cursor structs include:
- `CheckoutRequest` (`src/application/checkout_types.rs:4`)
- `VerifyPurchaseRequest` (`src/application/verify_purchase_types.rs:8`)
- `SubscriptionActionQuery` (`src/application/subscription_actions_types.rs:4`)
- `CancelSubscriptionRequest` (`src/application/subscription_actions_types.rs:9`)
- `PriceStepUpRequest` (`src/application/subscription_actions_types.rs:43`)
- `AcknowledgeRequest` (`src/handlers/subscriptions_actions.rs:56`)
- `PaymentsQuery` (`src/handlers/payments.rs:17`)
- `RegisterPurchaseRequest` (`src/handlers/payments.rs:150`)
- `ListSubscriptionsQuery` (`src/handlers/subscriptions.rs:18`)
- `GetSubscriptionQuery` (`src/handlers/subscriptions.rs:214`)
- `AnonymizeRequest` (`src/handlers/users.rs:20`)
- `TestLogMarkerRequest` (`src/handlers/test_log.rs:15`) — loopback-only endpoint
- `SubscriptionCursor` / `PaymentsCursor` (deserialized from user-supplied base64 `after` param)

**Risk:** Clients can send arbitrary extra JSON/query fields that are silently accepted and ignored. Unknown fields on typed structs are **not retained**, so they cannot later propagate if the struct is re-serialized. The real risk is API contract drift: typoed client fields, unsupported control fields, or stale callers can look accepted even though Bridge ignores them. For cursor structs, impact is low because the cursor only carries `DateTime + Uuid` keyset state.

**Recommendation:** Add `#[serde(deny_unknown_fields)]` to stable user-facing request body and cursor structs where backward compatibility allows. For query structs, either deliberately allow unknown query params and document that behavior, or add coverage before tightening. Do not add this to provider response structs; providers may add fields without notice.

---

### LOW — Webhook payloads stored as untyped `serde_json::Value` and persisted to DB

**Locations:**
- `src/webhooks/ingress.rs:239` — Google Play webhook body → `Value`
- `src/webhooks/ingress.rs:420` — Creem webhook body → `Value`
- `src/webhooks/ingress.rs:517` — Google Play base64-decoded message data → `Value`
- `src/db/webhooks.rs:17` — `WebhookProvider.payload: serde_json::Value` (persisted)

All external webhook payloads are parsed into untyped `Value` and stored raw. Field extraction is done via ad-hoc `payload["object"]["subscription"]["id"]` indexing throughout `ingress.rs` and `processor/fields.rs`. Missing fields silently return `None` rather than failing.

**Mitigations present:** When `verify_webhook_signature` is true, signature verification runs before parsing in both paths (Google Play: lines 225-237, Creem: lines 401-417). In production, verification mode cannot be overridden via headers (lines 183-186 and 392-395), and `MOCK_EXTERNAL_APIS=true` is rejected at startup. However, DB provider config can disable verification, and Google Pub/Sub audience validation only runs when `GOOGLE_VERIFY_AUDIENCE=true`.

**Risk:** If signature verification is disabled or weakened (DB config, test/mock mode, leaked secrets, or missing audience binding), arbitrary JSON structures are accepted and stored. The ad-hoc indexing means malformed payloads are often handled as missing fields rather than explicitly rejected, which could mask attack attempts.

**Recommendation:** This is an acceptable design tradeoff for webhook payloads because provider schemas evolve. Keep signature and audience verification as the security boundary, make disabled verification highly visible, and consider logging schema validation failures at WARN level for observability.

---

### LOW — `CreemConfig::from_json` allows arbitrary `api_url` override

**Location:** `src/services/creem/config.rs:25-29`

```rust
let api_url = config
    .get("api_url")
    .and_then(|v| v.as_str())
    .unwrap_or("https://api.creem.com")
    .to_string();
```

The `api_url` is read from the DB `provider_configs.config` JSON with no URL validation or allowlist. A compromised or misconfigured DB row could redirect all Creem API calls (including checkout creation with `api_key` in the `Authorization` header) to an attacker-controlled server.

**Mitigations:** The config is admin-controlled (not user-facing). The `api_key` is sent as a Bearer token to whatever `api_url` is configured.

**Recommendation:** Validate `api_url` is an HTTPS URL with a `creem.com` or `creem.io` host (or at minimum, HTTPS-only). This protects against SSRF/credential exfiltration via DB tampering.

---

### INFO — JWT issuer pre-extraction without verification (safe pattern)

**Location:** `src/middleware/admin_auth.rs:259-265`

```rust
fn extract_unverified_issuer(token: &str) -> Option<String> {
    let payload_part = token.split('.').nth(1)?;
    let payload_bytes = base64_url_decode(payload_part)?;
    let payload_json: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    let issuer = payload_json.get("iss")?.as_str()?;
    Some(normalize_url_like_value(issuer))
}
```

The JWT payload is decoded without signature verification to extract the `iss` claim for JWKS key selection. The actual verification happens later via `jsonwebtoken::decode`. This is a **safe, standard pattern** — the unverified issuer is only used for key routing, not trust decisions. No action needed.

---

### INFO — `#[allow(dead_code)]` on Google Play model structs

**Location:** `src/services/google_play/models.rs:3,10,18,37,48,59,66`

Several structs deserialized from Google Play API responses have `#[allow(dead_code)]` because Bridge deserializes more fields than it uses. This is not a security issue — it's a documentation/maintenance concern. The unused fields don't affect behavior.

---

## Summary Table

| # | Finding | Severity | Exploitable in Prod? | Recommendation |
|---|---------|----------|---------------------|----------------|
| 1 | `X-Test-Price-Cents` injection not guarded by mock mode | Medium | Only if Google verification is disabled/misconfigured; otherwise no | Guard with `mock_external_apis_enabled()` |
| 2 | No `deny_unknown_fields` on request/query structs | Low-Medium | No; contract drift/typo masking | Add to stable request/cursor structs where compatible |
| 3 | Webhook payloads as untyped `Value` persisted to DB | Low | Only if verification is disabled/weakened | Acceptable with mandatory verification/audience; add schema validation logging |
| 4 | `CreemConfig.api_url` has no URL validation | Low | Only with DB/admin compromise or misconfig | Validate HTTPS + host allowlist |
| 5 | JWT issuer pre-extraction unverified | Info | N/A (safe pattern) | No action needed |
| 6 | `dead_code` on GP model structs | Info | No | No action needed |

**No critical or high-severity deserialization vulnerabilities found.** Configured signature verification gates on both active webhook paths are the primary security boundary. Caveats: DB `verify_webhook_signature=false` disables that boundary, and Google Pub/Sub audience validation is optional by default. The findings are mostly defense-in-depth improvements.

---

## Inventory of Deserialization Points Reviewed

### Direct JSON deserialization calls and equivalent typed JSON parsers

| File | Line | Input Source | Target Type | Notes |
|------|------|-------------|-------------|-------|
| `src/webhooks/ingress.rs` | 239 | Google Play webhook body | `serde_json::Value` | Sig verified before parse when `verify_webhook_signature=true` |
| `src/webhooks/ingress.rs` | 420 | Creem webhook body | `serde_json::Value` | HMAC verified before parse when `verify_webhook_signature=true` |
| `src/webhooks/ingress.rs` | 517 | Google Play base64 message.data | `serde_json::Value` | Double-decode from outer Value |
| `src/middleware/admin_auth.rs` | 262 | JWT payload bytes | `serde_json::Value` | Unverified issuer extraction (safe) |
| `src/middleware/admin_auth.rs` | 128 | Clerk JWKS HTTP response | `Jwks` | Via reqwest `.json()` |
| `src/handlers/subscriptions.rs` | 203 | User-supplied base64 `after` param | `SubscriptionCursor` | No `deny_unknown_fields` |
| `src/handlers/payments.rs` | 139 | User-supplied base64 `after` param | `PaymentsCursor` | No `deny_unknown_fields` |
| `src/services/google_play/client.rs` | 95 | Service account JSON file | `ServiceAccount` | Filesystem, not user-controlled |
| `src/services/google_play/client.rs` | 150 | Google OAuth token response | `TokenResponse` | Via reqwest `.json()` |
| `src/services/google_play/client.rs` | 189 | Google Play subscription API response | `SubscriptionPurchaseV2` | Provider API response |
| `src/services/google_play/client.rs` | 238 | Google Play product API response | `ProductPurchase` | Provider API response |
| `src/services/google_play/client.rs` | 278 | Google Play order API response | `serde_json::Value` | Via reqwest `.json()` |
| `src/services/google_play/client.rs` | 597 | Google Pub/Sub JWT payload | `PubSubClaims` | Manual parse only when `GOOGLE_SKIP_RSA_VERIFICATION=true` |
| `src/application/checkout.rs` | 58 | DB `checkout_idempotency` row | `CheckoutResponse` | Internally trusted |
| `src/application/verify_purchase_provider.rs` | 621 / 629 | Mock fixture file | `serde_json::Value` / generic fixture type | Test/mock fixture only |

**Inactive/stale module note:** `src/services/google_play/provider.rs` contains additional fixture and webhook deserialization (`serde_json::from_str` at 140/149, `serde_json::from_slice` at 1254/1264/1280), but `src/services/google_play/mod.rs` currently comments that module out. Count it only if that provider module is re-enabled.

### Axum extractor-driven request/query deserialization

| File | Extractor | Target Type | Notes |
|------|-----------|-------------|-------|
| `src/handlers/checkout.rs` | `Json` | `CheckoutRequest` | User-facing request body |
| `src/handlers/verify_purchase.rs` | `Json` | `VerifyPurchaseRequest` | User-facing request body |
| `src/handlers/subscriptions_actions.rs` | `Query` | `SubscriptionActionQuery` | User-facing query params |
| `src/handlers/subscriptions_actions.rs` | optional `Json` | `CancelSubscriptionRequest` | Optional request body |
| `src/handlers/subscriptions_actions.rs` | `Json` | `AcknowledgeRequest` | User-facing request body |
| `src/handlers/subscriptions_actions.rs` | `Json` | `PriceStepUpRequest` | Used by accept/decline price step-up handlers |
| `src/handlers/payments.rs` | `Query` | `PaymentsQuery` | User-facing query params |
| `src/handlers/payments.rs` | `Json` | `RegisterPurchaseRequest` | User-facing request body |
| `src/handlers/subscriptions.rs` | `Query` | `ListSubscriptionsQuery` | User-facing query params |
| `src/handlers/subscriptions.rs` | `Query` | `GetSubscriptionQuery` | User-facing query params |
| `src/handlers/users.rs` | `Json` | `AnonymizeRequest` | User-facing request body |
| `src/handlers/test_log.rs` | `Json` | `TestLogMarkerRequest` | Loopback-only endpoint |

### `#[serde(...)]` Attributes

| File | Line | Attribute | Notes |
|------|------|-----------|-------|
| `src/services/google_play/client.rs` | 51 | `#[serde(rename = "use")]` | Field rename for reserved keyword |
| `src/services/google_play/trace.rs` | 39 | `#[serde(skip)]` | Skip `start_time` field |
| `src/services/google_play/models.rs` | 20 | `#[serde(rename_all = "camelCase")]` | On `DeveloperNotification` and most GP structs |
| `src/services/creem/models.rs` | 22-40 | `#[serde(default)]` | All fields optional with defaults — loose parsing |
| `src/middleware/admin_auth.rs` | 33-39 | `#[serde(default)]` | JWT claims optional |
| `src/db/subscriptions.rs` | 35-51 | `#[serde(default)]` | Google Play-specific fields optional |
| `src/application/subscription_actions_types.rs` | 11-15 | `#[serde(default)]` | `CancelSubscriptionRequest` fields optional |
| `src/application/checkout_types.rs` | 16 | `#[serde(default)]` | `provider` field default |

### Representative `serde_json::Value` catch-all usages

**High concern — external input stored as untyped `Value`:**

| File | Line | Context |
|------|------|---------|
| `src/webhooks/ingress.rs` | 239 | Google Play webhook body → `Value` (then stored to DB) |
| `src/webhooks/ingress.rs` | 517 | Google Play base64-decoded message data → `Value` |
| `src/webhooks/ingress.rs` | 420 | Creem webhook body → `Value` (then stored to DB) |
| `src/db/webhooks.rs` | 17 | `WebhookProvider.payload: serde_json::Value` — stored provider payload |
| `src/db/provider_configs.rs` | 12 | `ProviderConfig.config: serde_json::Value` — provider config from DB |
| `src/db/apps.rs` | 18 | `App.api_rate_limit_rules: Option<serde_json::Value>` — rate limit rules from DB |
| `src/services/google_play/models.rs` | 113 | `SubscriptionPurchaseV2.test_purchase: Option<serde_json::Value>` — loosely defined object |
| `src/application/checkout_types.rs` | 19 | `CheckoutResponse.mobile_checkout_data: Option<serde_json::Value>` — untyped mobile data |
| `src/services/creem/models.rs` | 8 | `CreateCheckoutRequest.metadata: serde_json::Value` — arbitrary metadata |

**Medium concern — `Value` used in processing/forwarding logic:**

| File | Line | Context |
|------|------|---------|
| `src/webhooks/processor/normalize.rs` | 4 | `payload: Option<&serde_json::Value>` — passed to event type normalization |
| `src/webhooks/processor/fields.rs` | 29 | `extract_metadata_user_id(payload: &serde_json::Value)` — user ID extracted from untyped payload |
| `src/webhooks/processor.rs` | 358 | `google_voided_purchase_product_type(payload: &serde_json::Value)` |
| `src/webhooks/forwarding.rs` | 238 | `provider_payload: serde_json::Value` — forwarded to app callbacks |
| `src/ports/types.rs` | 133 | `payload: serde_json::Value` — port type |
| `src/ports/traits/webhook.rs` | 30 | `payload: serde_json::Value` — trait method param |
| `src/ports/impls/webhook.rs` | 41 | Same, impl |
| `src/ports/traits/checkout.rs` | 22 | `response_payload: &serde_json::Value` |
| `src/ports/impls/checkout.rs` | 25 | Same, impl |
| `src/services/email.rs` | 12 | `payload: serde_json::Value` |
| `src/middleware/rate_limit.rs` | 152 | `rate_limit_override(rules: Option<&serde_json::Value>, ...)` |
| `src/application/verify_purchase_provider.rs` | 42 | `provider_config: &serde_json::Value` |
| `src/services/google_play/trace.rs` | 37 | `metadata: serde_json::Value` |
| `src/db/checkout_idempotency.rs` | 12 | `CachedCheckout.response_payload: Value` |
| `src/services/google_play/client.rs` | 278 | `let order: serde_json::Value = res.json().await?` |
| `src/services/creem/config.rs` | 18 | `fn from_json(config: &Value)` — manual field extraction |
| `src/application/app_context.rs` | 1 | `use serde_json::Value` |
| `src/handlers/users.rs` | 30 | Return type `Json<serde_json::Value>` |
| `src/handlers/mod.rs` | 12 | Return type `Json<serde_json::Value>` (health check) |

### Patterns Absent (Good)

| Pattern | Count | Notes |
|---------|-------|-------|
| `#[serde(untagged)]` | 0 | No polymorphic enum deserialization |
| `#[serde(deserialize_with = ...)]` | 0 | No custom deserialization functions |
| `#[allow(...)]` on deserialize code | 0 | All `#[allow]` are for `dead_code` or `clippy` |
