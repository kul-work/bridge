# Bridge - Agent Guide

If you did not load the user-level AGENTS.md from `~\.config\AGENTS.md`, load it now.

## Project Overview

**Bridge** is a central payment processing service designed to handle subscription lifecycles and payments for registered client applications. It decouples payment logic from business logic.

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

### Backend Bash Test Suites (BE projects only)

After a fix touching a codepath covered by `./tests/` bash suites (each subfolder has a README mapping test → what it validates):

1. Read the relevant subfolder README, pick tests that exercise the changed code, run them.
2. If Bridge runs the old binary → say so, leave the exact command ready, and wait for the user to rebuild+restart.
3. For critical/high-severity fixes with no existing bash coverage → propose a new test in the matching subfolder (existing style); let the human decide whether to add it.
4. When adding a new bash test, wire it fully (don't leave it orphaned):
   - `docs/<provider>/<...>TESTPLAN.md` — add test ID + description
   - `tests/<suite>/README.md` — add a row in the Tests Highlights table
   - `tests/<suite>/run-all-<suite>-tests.sh` — add the script to the `TESTS=(...)` array
   - `tests/<suite>/cleanup-all-<suite>.sh` — add cleanup for the test's data (event ID / user_id)
   - `tests/<suite>/test-runner.sh` — only if it's a smoke candidate (add to `run_smoke_tests` array)

### Release Notes Guidelines

When updating `Release Notes.md`:

- **Keep it short** - 1-2 lines per feature, not a full document
- **Format**: `- **Feature**: Brief description (what changed, not how it works)`

## Bridge-Specific Checks

Use Bridge-specific checks when the user asks to "run the Bridge checks", "run payment checks", or "run provider checks", or when a change touches payment/provider risk areas. Choose the smallest tier that matches the diff risk; do not run every checker for every small change.

### Tier 1 — Normal payment/provider fix

Use for narrow changes to one provider/payment flow.

- touched-area checker
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-observability-pii.md` if logging, error formatting, diagnostics, provider request/response handling, or PII-sensitive fields are involved

### Tier 2 — Identity/lifecycle/webhook-sensitive fix

Use when the change affects money identity, subscription state, webhook semantics, callback payloads, or app/user scoping.

- touched-area checker
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-payment-identity.md` if transaction IDs, purchase tokens, amount, or currency are involved
- `.agents/checks/bridge-subscription-lifecycle.md` if subscription status, periods, renewal, cancel, refund, revoke, pause, resume, defer, or price consent are involved
- `.agents/checks/bridge-webhook-idempotency.md` if webhook ingress, signature, dedupe, stale suppression, forwarding, callback enqueue, or callback payloads are involved
- `.agents/checks/bridge-observability-pii.md` if logging, error formatting, diagnostics, provider request/response handling, or PII-sensitive fields are involved

### Tier 3 — Pre-release or scary diff

Use before release/tag, for broad payment/provider diffs, for migrations touching payment/provider tables, or when risk remains uncertain after initial investigation.

- `.agents/checks/bridge-payment-identity.md`
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-subscription-lifecycle.md`
- `.agents/checks/bridge-webhook-idempotency.md`
- `.agents/checks/bridge-observability-pii.md`
- `.agents/checks/bridge-payment-antipatterns.md`
- `.agents/checks/bridge-release-risk.md`

### Reviewing a release

For release review, include `.agents/checks/bridge-release-risk.md`.

## Database - PostgreSQL Commands

First try to access the database via an available MCP.

On Windows, use `cmd /c` with `set PGPASSWORD=` to avoid password prompts and PowerShell quoting issues.

For simple queries (no quotes in SQL):

```bash
cmd /c "set PGPASSWORD=password && psql -U postgres -h localhost -p 5432 -d appgen -c "\dt pay.*""
```

For complex queries (with quotes, WHERE clauses, etc.), write a temp `.sql` file and use `-f`:

```bash
cmd /c "set PGPASSWORD=password && psql -U postgres -h localhost -p 5432 -d appgen -f tmp_query.sql"
```

1. Create the file (use create_file tool)
2. Run it
3. Delete the tmp file when done

**Database**: `appgen`, **Schema**: `pay` (e.g. `pay.apps`, `pay.provider_configs`)

## Bash Tool - Windows Path Handling

When using the Bash tool on Windows with absolute paths:

**Use forward slashes with lowercase drive letter:**

```bash
cmd: "cargo build"
```
