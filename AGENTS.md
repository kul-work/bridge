# Bridge - Agent Guide

## Project Overview

**Bridge** is a central payment processing service designed to handle subscription lifecycles, payments, and agent micropayments for all Tyde applications. It decouples payment logic from business logic.

**Tech Stack:**
- **Backend**: Rust + Axum + Tokio
- **Database**: PostgreSQL (SQLx)
- **API Support**: Multi-provider registry (Google Play, Creem, LemonSqueezy, Coinbase)
- **Security**: Double-ended HMAC validation on callbacks, provider signature checking.

## Architecture & Code Organization

**Backend Structure:**
- `src/main.rs` - Server entrypoint (Axum setup)
- `src/handlers/` - HTTP endpoint handlers (checkout, verification, subscriptions)
- `src/services/` - Provider integration modules (creem, google_play, etc.)
- `src/webhooks/` - Webhook ingress + processing
- `src/db/` - Database modules (queries separated by domain)
- `src/config.rs` - Environment config reader
- `migrations/` - PostgreSQL schema migrations

**Key Features:**
- **Administration**: Separate Admin UI secured by Tyde's internal Clerk organization.
- **Provider Normalization**: Maps canonical states across payment provider updates.
- **Idempotent Ingress Logging**: Prevents duplication hacks at webhook boundaries.
- **Webhook Sub-delivery Retries**: Guaranteed status flowback to apps using a 3-strike strategy.
- **Sub-schedule Reconciliation**: Heavy background reconciliation jobs verifying polling drift statuses.

## Code Style Guidelines

- **Language**: Rust (Edition 2021)
- **Framework**: Axum web framework with Tower middleware
- **Error Handling**: Use `thiserror` for custom errors, `anyhow` for context
- **Async**: Tokio runtime, use `#[tokio::test]` for async tests
- **Database**: SQLx with PostgreSQL, parameterized queries
- **Naming**: `snake_case` for functions/variables, `PascalCase` for types
- **Imports**: Group by std, external crates, then local modules
- NEVER use `cargo fmt` - is shit.

## 💡 Developer Principles (Guidance)

- **Minimize PII Storage**: Avoid recording general user PII (emails, names) unnecessarily in Bridge DB. Rely on opaque mappings like `external_user_id` from client apps.

- **Idempotency first**: Validate `webhook_log` before mutating states inside subscription layers to safe-guard duplication race-conditions.
- **Stale Event Suppression**: Avoid overwriting newer status updates with older triggers using high-water point comparison (`timestamp_epoch_ms`).
- **Minimal abstractions**: Don't create layers of abstraction until proven necessary (K.I.S.S.).

## Code Change Requests - Guidelines

### Investigation vs. Implementation

**CRITICAL:** When user prompt says "investigate", "what's happening?", "check", or "look into":

- DO NOT touch code. DO NOT ask clarifying questions. DO NOT implement.
- Read files, search, analyze, present findings ONLY.
- If user wants the fix applied, they will explicitly say so.

### How to Request Code Changes

Structure as:
1. **Current behavior** - what exists now
2. **Problem** - what's broken/missing
3. **Constraint** - what pattern should it follow?
4. **Success criteria** - how do I know it's done right?

---

## Database - PostgreSQL Commands

When running `psql` commands, always include the password via `PGPASSWORD` environment variable to avoid getting stuck at password prompts:

```bash
cmd: "PGPASSWORD=password psql -U postgres -h localhost -p 5432 -d bridge -c 'SELECT * FROM apps;'"
```
