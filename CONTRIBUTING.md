# Contributing to Bridge

Thanks for helping improve Bridge. This document is the short path from clone to a reviewable change.

## Before you start

1. Read [README.md](README.md) for setup.
2. For behavior or architecture changes, also read:
   - [DESIGN.md](DESIGN.md)
   - [INVARIANTS.md](INVARIANTS.md)
3. Prefer the smallest change that fixes the problem. Match existing style; do not reformat unrelated code.

Payment, webhook, subscription, and identity paths are high risk. When those areas move, expect extra scrutiny (see [docs/INDEX.md](docs/INDEX.md) and the checks under `.agents/checks/` if you use them).

## Development setup

```bash
cp .env.sample .env
# create DB + roles (docs/DB_ONBOARDING.md)
sqlx migrate run --database-url "$ADMIN_DATABASE_URL"
cargo run
```

Useful defaults for local work:

```env
ENVIRONMENT=development
MOCK_EXTERNAL_APIS=true
EMAIL_PROVIDER=mock
ENABLE_BACKGROUND_JOBS=true
```

Never set `MOCK_EXTERNAL_APIS=true` in production — startup rejects it.

## Build and test

### Rust

```bash
cargo check
cargo clippy -- -D warnings
cargo test --lib
```

Integration-style Rust tests may require Postgres and a prepared schema (see CI in `.github/workflows/ci.yml`).

### Shell suites

Provider and contract suites live under `tests/`:

| Suite | Path | Notes |
|-------|------|--------|
| Admin | `tests/admin/` | Clerk mock / admin API |
| Creem | `tests/creem/` | Creem billing flows |
| Google Play (GPBI) | `tests/gpbi/` | Google Play billing flows |
| Contract / isolation | `tests/cti/` | Cross-app isolation + contracts |
| Security | `tests/security/` | Cross-app read isolation |

Each suite has a `README.md` and a `run-all-*.sh` (or similar) entrypoint. Machine-specific secrets stay in suite-local `.env` files (gitignored), not in `globals.cfg`.

CI runs a fast gate on every push; heavier provider suites run on the nightly workflow.

## Coding guidelines

- **Rust edition 2021**, Axum + Tokio, SQLx parameterized queries.
- **Errors**: `thiserror` for typed errors, `anyhow` for context where already used.
- **Naming**: `snake_case` functions/vars, `PascalCase` types.
- **K.I.S.S.**: one clear purpose per function/module; no new abstraction layers without need.
- **Idempotency**: webhook ingress must stay safe under retries and duplicates.
- **PII**: do not log raw purchase tokens, API keys, HMAC secrets, or full provider callback bodies.
- Do **not** run `cargo fmt` as a bulk reformat of untouched code in this project’s history of convention; keep diffs surgical.

## Pull requests

1. One logical change per PR when practical.
2. Describe **what** changed and **why**, especially for money/webhook semantics.
3. Link or mention the invariant/doc you relied on when behavior is subtle.
4. Add or extend tests for the path you touch (Rust unit test and/or the matching bash suite).
5. Do not commit secrets, real service accounts, or local `.env` files.

### Suggested PR checklist

- [ ] `cargo check` / focused tests pass
- [ ] No new logging of secrets or raw provider payloads
- [ ] App-scoping / RLS considered if the change touches multi-tenant data
- [ ] Docs updated if the public API, config surface, or operator behavior changed

## Documentation

- User-facing API changes → `docs/API_CONTRACT.md`
- Config surface → `docs/CONFIGURATION.md`
- Architecture / guarantees → `DESIGN.md`, `INVARIANTS.md`
- Release notes style (if maintainers ask) → short bullets in `Release Notes.md`

## Security issues

Do **not** open a public issue for vulnerabilities. Follow [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions are licensed under the same [MIT License](LICENSE) that covers this project.
