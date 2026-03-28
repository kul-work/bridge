# Bridge — Multi-App Payment Gateway

**Bridge** is a central payment processing service designed to handle subscription lifecycles, payments, and agent micropayments for all Tyde applications. It decouples complex payment logic, provider webhooks, and ledger auditing mechanics from core business applications. 

It operates as a private, centralized gateway (e.g., `pay.tydecode.com`) serving approved Tyde application instances (such as **hiha.app**).

### Tech Stack

- **Backend**: Rust + Axum + Tokio
- **Database**: PostgreSQL (SQLx)
- **API Support**: Multi-provider registry (Creem, LemonSqueezy, Google Play, Apple IAP, Coinbase)
- **Security**: Double-ended HMAC validation on callbacks, explicit provider signature cryptographic verification


## Core Principles

- **Opaque Identifiers**: Bridge prioritizes keeping general user PII out of the core database, relying on `external_user_id` that client apps map back to users. It supports linked identifiers (like Agent emails) when required for Ledger tracking.

- **Non-Public**: It only serves approved Tyde client applications using secure API key authentication.
- **Provider Abstraction**: Normalizes events across providers (Google Play, Creem, Apple, LemonSqueezy) into a common format for apps.

## Responsibilities

- **App Registry System**: Manages registered apps, credential hashing, and callbacks.
- **Subscription Lifecycle System**: Source of truth for recurring billing states.
- **Webhook Ingress Handlers**: Safely absorbs provider webhooks with full idempotency checks.
- **Webhook Sub-delivery Forwarding**: Delivers actionable notifications securely to app backends with retry strategy.
- **Agent Micro-payment Ledgers**: Tracks virtual credits and atomic scoped reservations for automated micro-payment mechanics.

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
# Application Encryption key (AES-GCM for provider keys)
ENCRYPTION_KEY=your-secure-key
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

- `GET /health` — Diagnostics
- `POST /api/v1/checkout` — Initiate session with provider
- `POST /api/v1/verify-purchase` — Mobile receipt verification
- `GET /api/v1/subscriptions` — List recurring billing status 
- `POST /api/v1/agent/token` — Scoped micro-payments tokens
- `POST /api/v1/agent/charge` — Atomically consume funds