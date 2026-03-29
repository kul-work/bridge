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

## Database - PostgreSQL Commands

On Windows, use `cmd /c` with `set PGPASSWORD=` to avoid password prompts and PowerShell quoting issues.

For simple queries (no quotes in SQL):
```bash
cmd /c "set PGPASSWORD=password&& psql -U postgres -h localhost -p 5432 -d appgen -c "\dt pay.*""
```

For complex queries (with quotes, WHERE clauses, etc.), write a temp `.sql` file and use `-f`:
```bash
# 1. Create the file (use create_file tool)
# 2. Run it:
cmd /c "set PGPASSWORD=password&& psql -U postgres -h localhost -p 5432 -d appgen -f c:\share\tyde\bridge\tmp_query.sql"
# 3. Delete the tmp file when done
```

**Database**: `appgen`, **Schema**: `pay` (e.g. `pay.apps`, `pay.provider_configs`)

## Bash Tool - Windows Path Handling

When using the Bash tool on Windows with absolute paths:

**Use forward slashes with lowercase drive letter:**

```bash
cmd: "cd c:/share/tyde/bridge && cargo build"
```
