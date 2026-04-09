# Bridge — Multi-App Payment Gateway

**Bridge** is a central payment processing service designed to handle subscription lifecycles, payments, and agent micropayments for all Tyde applications. It decouples complex payment logic, provider webhooks, and ledger auditing mechanics from core business applications. 

It operates as a private, centralized gateway (e.g., `pay.tydecode.com`) serving approved Tyde application instances (such as **hiha.app**).

### Tech Stack

- **Version**: 0.1.2
- **Backend**: Rust + Axum + Tokio
- **Database**: PostgreSQL (SQLx)
- **API Support**: Multi-provider registry (Creem, LemonSqueezy, Google Play, Coinbase)
- **Security**: Double-ended HMAC validation on callbacks, explicit provider signature cryptographic verification, rate limiting, and API key authentication.

## Core Principles

- **Opaque Identifiers**: Bridge prioritizes keeping general user PII out of the core database, relying on `external_user_id` that client apps map back to users. It supports linked identifiers (like Agent emails) when required for Ledger tracking.
- **Non-Public**: It only serves approved Tyde client applications using secure API key authentication.
- **Provider Abstraction**: Normalizes events across providers into a common format for apps.
- **Idempotency First**: Strict webhook deduplication and state-change guards to prevent race conditions or duplicate processing.

## Responsibilities

- **App Registry System**: Manages registered apps, credential hashing, and callbacks.
- **Subscription Lifecycle System**: Source of truth for recurring billing states, including upgrades, downgrades, and linked subscriptions.
- **Webhook Ingress Handlers**: Safely absorbs provider webhooks with full idempotency checks and event ordering.
- **Webhook Sub-delivery Forwarding**: Delivers actionable notifications securely to app backends with a 3-strike retry strategy.
- **Agent Micro-payment Ledgers**: Tracks virtual credits and atomic scoped reservations for automated micro-payment mechanics.
- **Reconciliation Engine**: Background workers verifying provider status polling drift and self-healing subscription states.

## Administration

Bridge includes a built-in **Admin Dashboard** (secured by Tyde's internal auth) for monitoring:
- Registered applications and their configurations.
- Webhook ingress logs and delivery status.
- Manual webhook retry and reconciliation triggers.
- Global system health and background worker status.

## Prerequisites

- **Language**: Rust 1.75+
- **Database**: PostgreSQL 17+
- **Administration**: Secured by Tyde’s internal Admin UI (Clerk authorized).

## Quickstart

### 1. Setup Environment

```bash
cp .env.sample .env
```

Set at minimum:
```env
# Database configuration
DATABASE_URL=postgresql://user:password@localhost/appgen
```

### 2. Run Database Migrations

```bash
sqlx migrate run --database-url postgresql://bridge_admin:password@localhost:5432/appgen
```

Or if using a `.env` file with `DATABASE_URL`:

```bash
sqlx migrate run
```

### 3. Run Application

```bash
cargo run
```

Backend serves on port `3000` (default).

## Authentication

- **App → Bridge**: Requires API key (`Authorization: Bearer sk_app_...`)
- **Bridge → App**: HMACS payload using `X-Pay-Signature` for safe callback handling.

## API Snapshot Overview

### Subscriptions & Payments
- `POST /api/v1/checkout` — Initiate session with provider
- `POST /api/v1/verify-purchase` — Mobile receipt verification
- `GET /api/v1/subscriptions` — List recurring billing status 
- `GET /api/v1/subscriptions/:id` — Get specific subscription details
- `POST /api/v1/subscriptions/:id/cancel` — Cancel subscription
- `POST /api/v1/subscriptions/:id/resume` — Resume subscription
- `POST /api/v1/subscriptions/:id/acknowledge` — Acknowledge purchase (Google Play compliance)
- `POST /api/v1/subscriptions/:id/portal` — Create billing portal URL
- `POST /api/v1/subscriptions/:id/price-step-up/accept` — Accept price changes
- `POST /api/v1/subscriptions/:id/price-step-up/decline` — Decline price changes
- `GET /api/v1/payments` — Query payment history
- `POST /api/v1/purchase/register` — Register external/one-time purchases

### Agent & Credits
- `GET /api/v1/agent/balance` — Check virtual credit balance
- `POST /api/v1/agent/token` — Scoped micro-payments tokens
- `POST /api/v1/agent/charge` — Atomically consume funds
- `POST /api/v1/agent/topup` — Manually add/top-up credits

### User & Privacy
- `POST /api/v1/users/:id/anonymize` — GDPR/Privacy anonymization
- `GET /api/v1/users/:id/data-export` — GDPR data export

### System
- `GET /health` — Diagnostics
- `/admin/*` — Administration restricted routes

