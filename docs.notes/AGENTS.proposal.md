# Bridge - Agent Guide

If you're running in Crush, also read the rules from `.\.crush\AGENTS.md`.

## Project Overview

**Bridge** is a central payment processing service designed to handle subscription lifecycles and payments for all Tyde applications. It decouples payment logic from business logic.

**Tech Stack:**
- **Backend**: Rust + Axum + Tokio
- **Database**: PostgreSQL (SQLx)
- **API Support**: Multi-provider registry (Google Play, Creem)
- **Security**: Double-ended HMAC validation on callbacks, provider signature checking.

## Architecture & Code Organization

**Backend Structure:**
- `src/main.rs` - Server entrypoint (Axum setup)
- `src/handlers/` - HTTP endpoint handlers (checkout, verification, subscriptions, admin)
- `src/services/` - Provider integration (creem, google_play) and core logic (payment, email)
- `src/webhooks/` - Webhook ingress routing and status processing
- `src/db/` - Database modules (queries separated by domain)
- `src/middleware/` - Auth, Rate limiting, and tracing layers
- `src/ports/` - External trait definitions (optional abstraction layer)
- `src/application/` - Domain orchestrators and business rules
- `migrations/` - PostgreSQL schema migrations

**Key Features:**
- **Administration**: Integrated Admin Dashboard at `/admin` for monitoring apps and webhooks.
- **Provider Normalization**: Maps canonical states across payment provider updates.
- **Idempotent Ingress Logging**: Prevents duplication hacks at webhook boundaries using `webhook_log`.
- **Webhook Sub-delivery Retries**: Guaranteed status flowback to apps using a 3-strike strategy.
- **Background Workers**:
    - **Reconciliation**: Verifies polling drift statuses against provider APIs.
    - **Cleanup**: Housekeeping for old logs and temporary state.
    - **Price Step-up**: Manages Korea-specific price consent lifecycle.
    - **Pause Scheduler**: Handles delayed subscription pauses and resumes.

## Required Reading

Before making architectural or behavioral changes, read:
- `DESIGN.md` — system architecture and design decisions
- `INVARIANTS.md` — behavioral invariants that must not be violated

## Code Style Guidelines

- **Language**: Rust (Edition 2021)
- **Framework**: Axum web framework with Tower middleware
- **Error Handling**: Use `thiserror` for custom errors, `anyhow` for context
- **Async**: Tokio runtime, use `#[tokio::test]` for async tests
- **Database**: SQLx with PostgreSQL, parameterized queries
- **Naming**: `snake_case` for functions/variables, `PascalCase` for types
- **Imports**: Group by std, external crates, then local modules
- NEVER use `cargo fmt` - is shit.

## K.I.S.S. (Keep It Simple, Stupid)

- **Avoid over-engineering**: Implement the simplest solution that solves the problem
- **Readable > Clever**: Code that's easy to understand beats clever one-liners
- **Single responsibility**: Each function/module should have one clear purpose
- **No magic numbers/strings**: Always explain why values exist (use constants/comments)
- **Minimal abstractions**: Don't create layers of abstraction until proven necessary

## 💡 Developer Principles (Guidance)

- **Minimize PII Storage**: Avoid recording general user PII (emails, names) unnecessarily in Bridge DB. Rely on opaque mappings like `external_user_id` from client apps.
- **Idempotency first**: Validate `webhook_log` before mutating states inside subscription layers to safe-guard duplication race-conditions.
- **Stale Event Suppression**: Avoid overwriting newer status updates with older triggers using high-water point comparison (`timestamp_epoch_ms`).

## Code Change Requests - Guidelines

### Investigation vs. Implementation

**CRITICAL:** When user prompt says "investigate", "what's happening?", "check", or "look into":

- DO NOT touch code. DO NOT ask clarifying questions. DO NOT implement.
- Read files, search, analyze, present findings ONLY.
- If user wants the fix applied, they will explicitly say so.

Only implement if user says: "fix", "implement", "make", "change", or explicitly requests a solution.

### Before Asking for Changes

**Provide context:**

- What pattern/library already exists for this problem?
- Where in the architecture does it happen?
- Why is the existing approach not working for this case?

**Be explicit about constraints:**

- Should I follow existing patterns or introduce new approaches?
- Are there dependencies/libraries I must or must not use?
- Is there a specific "style" or architectural decision already made?

### How to Request Code Changes

Structure as:

1. **Current behavior** - what exists now
2. **Problem** - what's broken/missing
3. **Constraint** - what pattern should it follow?
4. **Success criteria** - how do I know it's done right?

Avoid:

- "Fix X in Y" (too vague)
- Asking me to decide architectural choices you haven't made

### Project Patterns

- **Consistency first:** Match existing code style before optimizing
- **Surgical changes:** Keep changes as minimal as possible to minimize risk and respect the history of working code. Never reformat working blocks (e.g., collapsing multi-line `if` or `match` blocks)

### Release Notes Guidelines

When updating `Release Notes.md`:

- **Keep it short** - 1-2 lines per feature, not a full document
- **Format**: `- **Feature**: Brief description (what changed, not how it works)`

## Payment/Provider Guardrails

Before changing payment/provider behavior, classify the task:

- **PARITY**: must match old HiHa behavior. Use `bridge-hiha-parity` first.
- **BRIDGE-ONLY**: intentionally differs because of Bridge architecture.
- **UNKNOWN**: stop and investigate before implementation.

Payment/provider behavior includes changes that affect any of:

- payment identity: `provider_transaction_id`, purchase token, order ID, amount, currency
- durable payment/subscription writes
- subscription status, period, pause, cancel, defer, refund, revoke, renew, price consent
- provider normalization or provider API translation
- webhook ingress, deduplication, stale suppression, forwarding, callback payloads
- app/user scoping, provider config lookup, RLS-sensitive query paths
- checkout or verify-purchase flows
- migrations touching `pay.*` payment/provider/subscription/webhook tables

Common files include, but are not limited to:

- `src/webhooks/**`
- `src/application/verify_purchase*`
- `src/application/checkout*`
- `src/application/subscription*`
- `src/db/payments.rs`
- `src/db/subscriptions.rs`
- `src/db/webhooks.rs`
- `src/db/provider_configs.rs`
- `src/db/checkout_idempotency.rs`
- `src/services/google_play/**`
- `src/services/creem/**`
- `src/services/provider_api.rs`
- `src/handlers/{checkout,payments,subscriptions,subscriptions_actions,verify_purchase}.rs`
- `src/ports/**/{payment,subscription,webhook,checkout}.rs`
- `migrations/*`

Then apply the relevant `.agents/checks/bridge-*.md` checkers:

- payment identity -> `.agents/checks/bridge-payment-identity.md`
- durable side effects / test assertions -> `.agents/checks/bridge-payment-side-effects.md`
- subscription status/lifecycle -> `.agents/checks/bridge-subscription-lifecycle.md`
- webhook dedup/stale/callback behavior -> `.agents/checks/bridge-webhook-idempotency.md`
- release classification -> `.agents/checks/bridge-release-risk.md`

For payment/provider bug fixes, always apply `.agents/checks/bridge-payment-side-effects.md` plus any checker matching the touched behavior. Every payment/provider bug fix must leave behind an assertion that would have caught it, or a documented reason why not.

Apply all Bridge payment checkers only when the user explicitly asks for Bridge checks, the fix crosses payment/subscription/webhook boundaries, the change is pre-release, or the classification remains uncertain after initial investigation.

For unclassifiable or cross-cutting tasks, fill the fallback intake template before implementing.

## Bridge-Specific Review Checks

When the user asks to "run the Bridge checks", "run Bridge-specific checks", "run payment checks", or "run provider checks", use the relevant check files in `.agents/checks/`.

For payment/provider changes, apply at least:

- `.agents/checks/bridge-payment-identity.md`
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-webhook-idempotency.md`
- `.agents/checks/bridge-subscription-lifecycle.md`
- `.agents/checks/bridge-payment-antipatterns.md`

For release review, also apply:

- `.agents/checks/bridge-release-risk.md`

## Database - PostgreSQL Commands

First try to access the database via an available MCP.

On Windows, use `cmd /c` with `set PGPASSWORD=` to avoid password prompts and PowerShell quoting issues.

For simple queries (no quotes in SQL):

```bash
cmd /c "set PGPASSWORD=password && psql -U postgres -h localhost -p 5432 -d appgen -c "\dt pay.*""
```

For complex queries (with quotes, WHERE clauses, etc.), write a temp `.sql` file and use `-f`:

```bash
cmd /c "set PGPASSWORD=password && psql -U postgres -h localhost -p 5432 -d appgen -f c:\share\tyde\bridge\tmp_query.sql"
```

1. Create the file (use create_file tool)
2. Run it
3. Delete the tmp file when done

**Database**: `appgen`, **Schema**: `pay` (e.g. `pay.apps`, `pay.provider_configs`)

## Bash Tool - Windows Path Handling

When using the Bash tool on Windows with absolute paths:

**Use forward slashes with lowercase drive letter:**

```bash
cmd: "cd c:/share/tyde/bridge && cargo build"
```
