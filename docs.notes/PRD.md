# HiHa Technical Architecture - Payments and Content Delivery

**Document Version:** 2.0  
**Last Updated:** March 2026  
**Status:** Implementation-Aligned (from current codebase)

---

## 1. Purpose

This is the single technical reference for splitting HiHa into:

1. A **Payment Middleware** service (billing, subscriptions, webhooks, entitlement state).

---

## 2. Current Deployment Topology

Currently runs as one Axum process (`src/main.rs`) serving:

1. Payment APIs (checkout, verify purchase, status, history, subscription actions).
2. Webhooks (`/webhooks/:provider`).
3. Background schedulers (pause transitions, reconciliation, consent expiry, cleanup jobs).

All domains share one PostgreSQL database and one `AppState` object.

---

## 3. Payment Components

1. Provider abstraction: `src/services/payment.rs` (`PaymentProvider` trait).
2. Providers: Creem, Google Play, LemonSqueezy, Coinbase.
3. Payment handlers under `src/handlers/payments/*`.
4. Webhook ingress and async processor under `src/webhooks/*`.
5. Payment and subscription DB logic in `src/db/payments.rs`, `src/db/subscriptions.rs`, `src/db/webhooks.rs`, `src/db/notifications.rs`, `src/db/agent_billing.rs`.
6. Clerk token verification and email resolution (`src/services/clerk.rs`).
7. Rate limiting (`src/db/rate_limits.rs`) used by payment endpoints.

---

## 4. External API Surface (Current)

### 4.1 Public Routes (No client-version middleware)

1. `GET /health`
2. `POST /api/v1/purchase/register` (still requires auth in handler)
3. `POST /api/v1/payment/verify` (still requires auth in handler)

### 4.2 Client API Routes (client-version middleware applied)

4. `GET /api/v1/subscription-status`
5. `GET /api/v1/payment/history`
6. `POST /api/v1/checkout`
7. `POST /api/v1/subscriptions/:subscription_id/cancel`
8. `POST /api/v1/subscriptions/:subscription_id/portal`
9. `POST /api/v1/subscriptions/:subscription_id/resume`
10. `POST /api/v1/subscription/price-step-up/accept`
11. `POST /api/v1/subscription/price-step-up/decline`
12. `POST /api/v1/notifications/payment-failure/acknowledge`
13. `GET /api/v1/notifications/history`

### 4.3 Webhook Route

1. `POST /webhooks/:provider`

---

## 5. Authentication and Access Model

### 5.1 Human Authentication

1. JWT is extracted from `Authorization: Bearer <token>`.
2. Clerk service verifies JWT against JWKS and issuer.
3. Email resolution chain: token claim -> users table cache -> Clerk API fallback.
4. User row is upserted/updated with email if needed.

### 5.2 Agent/402 Identity

1. Agent flow uses `X-Agent-Email` and optional `X-Payment-Token` or `X-Payment-Proof`.
2. Story endpoint can be used without Clerk JWT if 402 flow succeeds.
3. Joke endpoint currently allows anonymous requests (free path) after IP/global checks.

### 5.3 Client Version Enforcement

1. Middleware checks `X-Client-Version` only on client API routes.
2. Requests with `X-Agent-Email` bypass client version checks.
3. Public routes and webhook routes do not use client-version middleware.

---

## 6. Payment and Subscription Behavior

### 6.1 Checkout

1. `POST /api/v1/checkout` always uses Creem as default web provider.
2. `product_type` supports product selection (`offer`, `otp`, default recurring product).
3. Checkout creates provider session and returns redirect URL.

### 6.2 Mobile Verification (`POST /api/v1/payment/verify`)

1. Validates auth and rate limits.
2. Verifies purchase via selected provider (`provider` in payload).
3. Handles Google linking conflicts with `409 linking_required` response.
4. Persists payment and subscription in DB; acks Google purchases idempotently.
5. Returns `202 Accepted` for pending purchases, `200 OK` for active/complete.

### 6.3 Subscription Management

1. Status read from users + subscriptions tables (`/api/v1/subscription-status`).
2. Cancel supports provider-specific behavior and mode flags.
3. Resume and billing portal endpoints are provider-routed via stored subscription context.
4. Price step-up consent accept/decline endpoints are implemented and tied to Google Play flows.

### 6.4 HTTP 402 Agent Micropayment Flow

1. Implemented inline in content guard path (`try_402_payment`), not as separate public endpoint.
2. Uses agent balance, payment tokens, and optional Coinbase proof verification.
3. One-time token reserve protects against concurrency replay.
4. Note: `src/handlers/agent_402.rs` exists but is not currently wired to router.

---

## 7. Webhook Pipeline

### 7.1 Ingress and Idempotency

1. Provider signature verification is done first.
2. Webhook dedupe is persisted atomically in `webhooks` table.
3. If duplicate, processing is skipped.
4. Remaining business handling runs in a spawned async task.

### 7.2 Event Processing

1. Processor routes normalized event types to payment/subscription/OTP/agent handlers.
2. Google Play events map notification types into normalized internal event names.
3. Subscription writes use chronological guard via `last_event_time` to avoid stale-event regression.

### 7.3 Provider-Specific Notes

1. Creem supports metadata-based user binding (`metadata.user_id`) and rich event normalization.
2. Google Play verifies Pub/Sub JWT signature and optionally audience, with configurable strictness.
3. Coinbase `charge.confirmed` credits agent balance.
4. LemonSqueezy support exists but web checkout defaults to Creem.

---

## 8. Rate Limiting and Budget Controls

### 8.1 Implemented Layers

1. IP-based limiter (`ip:<address>` key in `rate_limits`).
2. User/per-endpoint limiter (`clerk_id` + endpoint key).

### 8.2 Defaults from Config

3. IP limit: `RATE_LIMIT_IP_MAX_REQUESTS` default `300/min` (via one-minute window logic).
4. Global LLM: `RATE_LIMIT_GLOBAL_LLM` default `500/min`.

### 8.3 Important Implementation Detail

The DB limiter currently uses a fixed 1-minute window constant (`RATE_LIMIT_DURATION_MINUTES_DEFAULT = 1`). `rate_limit_duration_minutes` exists in config but is not used by limiter logic.

---

## 9. Data Model (Current)

### 9.1 Core Tables

1. `users` - Clerk ID, email cache, premium flags and expiry.
2. `subscriptions` - normalized status + provider-specific Google fields + chronology guard.
3. `payments` - provider transaction records and acknowledgment tracking.
4. `webhooks` - idempotency ledger and payload audit.
5. `notifications` - transactional email delivery state.
6. `rate_limits` - request counters for IP/user/global keys.
7. `agent_credits`, `agent_transactions`, `agent_payment_tokens` - 402 balance/token subsystem.

### 9.2 Current Ownership Reality

Payment lifecycle updates write both `subscriptions` and `users.is_premium`/`premium_expires_at`, which couples payment and content entitlement logic through shared tables.

---

## 10. Background Jobs

### 10.1 Scheduler Jobs (`src/schedule.rs`)

1. Expired price step-up consent auto-cancel.
2. Rate-limit cleanup.
3. Google Play reconciliation polling (if configured and service-account file exists).

### 10.2 Pause Scheduler (`src/services/pause_scheduler.rs`)

1. Auto-transition scheduled pauses to paused state.
2. Cleanup orphan pending subscriptions.

---

## 11. Coupling Hotspots Blocking the Split

### 11.1 Entitlement Coupling

Content handlers call DB directly for premium checks and 402 reserve logic. This bypasses any payment-service boundary.

### 11.2 Identity Coupling

Both payment and content paths call Clerk verification and user upsert logic in-process.

### 11.3 Transactional Coupling

`verify_purchase` and webhook handlers update payments, subscriptions, user premium flags, and notifications in one service boundary today.

### 11.4 Rate-Limit Coupling

Payment and content endpoints share same limiter table and helper API.

---

## 12. Target Split Architecture

### 12.1 Payment Middleware Responsibilities

1. Provider integrations (Creem, Google Play, LemonSqueezy, Coinbase).
2. Checkout/session creation.
3. Mobile purchase verification.
4. Webhook ingress, verification, dedupe, lifecycle processing.
5. Subscription state machine and entitlement state.
6. 402 balances, token lifecycle, and Coinbase credits.
7. Payment notifications and reconciliation.
8. Clerk token validation strategy (centralized utility or trusted gateway identity headers).
9. Correlation IDs and trace propagation.
10. Shared API error model for content + payment domains.

---

**End of Document**
