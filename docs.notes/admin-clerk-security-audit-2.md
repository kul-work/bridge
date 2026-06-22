# Bridge Admin Clerk Security Audit

Audit date: 2026-06-22  
Scope: Bridge admin area with Clerk login, including route protection, Clerk JWT validation, admin HTML/JS, webhook payload viewing, manual webhook retry, manual background-job triggers, and admin configuration.

## Executive Summary

The public `/admin` HTML page is not the main security problem. A Clerk publishable key is public by design, and the page can be served unauthenticated as long as every sensitive read/action is protected server-side.

I did not find an obvious unauthenticated admin JSON route. The sensitive admin routes are grouped separately and protected by `admin_auth_middleware`.

The biggest risks are configuration and authorization hardening:

1. If `ADMIN_CLERK_ORG_ID` is omitted, Bridge accepts any valid user from the configured Clerk issuer.
2. Even when org enforcement is enabled, there is no admin role/permission check.
3. Admin mutating actions are powerful but lack rate limiting, concurrency protection, and actor audit trails.
4. The admin page handles bearer tokens while using a permissive inline-script CSP.
5. Webhook payload redaction is helpful but incomplete for PII/provider-specific secrets.

## Scope Reviewed

- `src/main.rs`
- `src/middleware/admin_auth.rs`
- `src/handlers/admin.rs`
- `templates/admin.html`
- `src/db/webhooks.rs`
- `src/ports/impls/admin.rs`
- `src/ports/traits/admin.rs`
- `src/ports/impls/webhook.rs`
- `src/ports/traits/webhook.rs`
- `.env.sample`
- `docs/CONFIGURATION.md`

## Positive Findings

### Admin JSON routes are protected server-side

The admin HTML route is public, but the JSON/action routes are defined separately and layered with `admin_auth_middleware`:

- `src/main.rs:169-188`

Protected routes include:

- `GET /admin/apps`
- `PATCH /admin/apps/:app_id/notes`
- `GET /admin/apps/:app_id/webhooks`
- `GET /admin/webhooks/:webhook_id/payload`
- `POST /admin/webhooks/:webhook_id/retry`
- `POST /admin/trigger-jobs`

### JWT validation has important baseline checks

The custom Clerk verifier checks:

- JWT shape and non-empty parts: `src/middleware/admin_auth.rs:223-234`
- `RS256` only, non-empty `kid`, no unsupported `crit`: `src/middleware/admin_auth.rs:243-257`
- issuer before JWKS fetch: `src/middleware/admin_auth.rs:156-164`
- signature against Clerk JWKS: `src/middleware/admin_auth.rs:166-187`
- expiration and `nbf`: `src/middleware/admin_auth.rs:266-278`
- non-empty subject: `src/middleware/admin_auth.rs:199-201`
- optional authorized-party allowlist: `src/middleware/admin_auth.rs:280-298`
- optional org enforcement: `src/middleware/admin_auth.rs:203-215`

### CSRF exposure is low for current admin APIs

Admin APIs require `Authorization: Bearer <token>` rather than ambient cookies:

- `src/middleware/admin_auth.rs:402-415`
- `templates/admin.html:391-397`

That does not eliminate XSS risk, but it makes ordinary cross-site form/image/script CSRF much less useful.

### Manual webhook retry no longer reopens forwarded deliveries

The previous dangerous retry behavior appears fixed. The reset query now only resets deliveries that are both unforwarded and dead-lettered:

- `src/db/webhooks.rs:187-215`

The UI only exposes retry for dead-lettered, unforwarded deliveries:

- `templates/admin.html:569-571`

## Findings

### Medium - CSP is too permissive for a token-bearing admin page

Evidence:

- CSP allows inline scripts and inline styles: `src/handlers/admin.rs:33-48`
- The page uses inline event handlers such as `onclick="signOut()"`: `templates/admin.html:213-233`
- Clerk JS is loaded from jsDelivr via `@5` without visible SRI/integrity pinning: `templates/admin.html:335-338`
- The inline script fetches Clerk session tokens and calls admin APIs: `templates/admin.html:384-397`

Impact:

Current rendering mostly avoids obvious direct XSS sinks, but any future XSS bug on this page can steal or use Clerk bearer tokens. Because CSP allows `'unsafe-inline'`, CSP provides limited containment if a script injection bug appears.

Recommendation:

- Move admin JS into a same-origin static asset or serve per-request nonces.
- Remove inline `onclick` handlers and bind events from JS.
- Replace `'unsafe-inline'` with nonce/hash-based script execution.
- Pin/self-host/version-lock the Clerk script where practical.
- Keep the existing good CSP controls: `frame-ancestors 'none'`, `base-uri 'none'`, and `object-src 'none'`.

### Low - Admin responses do not set cache-control headers

Evidence:

- Admin HTML gets CSP and related browser security headers: `src/handlers/admin.rs:33-66`
- There is no `Cache-Control` admin security header in that function, and admin JSON responses are plain `Json` responses.

Impact:

Browsers and intermediaries should not cache admin pages, app metadata, webhook payloads, or job results. The practical risk depends on deployment proxy behavior, but payment admin surfaces should be explicit.

Recommendation:

- Add `Cache-Control: no-store` for `/admin` HTML and admin JSON/action responses.
- Consider `Pragma: no-cache` only for legacy compatibility if needed.

### Low - JWKS cache TTL is long and does not refresh on `kid` miss

Evidence:

- JWKS TTL is seven days: `src/middleware/admin_auth.rs:24-25`
- Cached JWKS is reused until TTL expiry: `src/middleware/admin_auth.rs:114-121`
- A missing `kid` fails directly against the cached JWKS: `src/middleware/admin_auth.rs:166-174`

Impact:

Normal Clerk key rotation can cause new sessions to fail until the cache expires. If a key is revoked due to compromise, Bridge may keep accepting tokens signed by the old cached key until TTL expiry.

Recommendation:

- Reduce JWKS TTL or honor upstream cache headers.
- On `kid` miss, perform one forced JWKS refresh before rejecting.
- Add a small backoff so repeated bad `kid` values do not cause fetch amplification.

### Low - Invalid admin tokens are parsed/logged without size and log-safety limits

Evidence:

- The middleware accepts any non-empty bearer token before parsing: `src/middleware/admin_auth.rs:402-415`
- Rejection logs can include attacker-controlled issuer, `kid`, or `azp` values: `src/middleware/admin_auth.rs:159-163`, `src/middleware/admin_auth.rs:166-171`, `src/middleware/admin_auth.rs:292-295`, `src/middleware/admin_auth.rs:427-429`

Impact:

Unauthenticated callers can submit very large JWT-looking values to consume CPU/memory during base64/JSON parsing and pollute logs with oversized attacker-controlled strings.

Recommendation:

- Reject Authorization headers above a conservative max length before parsing.
- Truncate/sanitize attacker-controlled claim/header values in logs.
- Put unauthenticated IP rate limiting in front of admin JWT verification.
