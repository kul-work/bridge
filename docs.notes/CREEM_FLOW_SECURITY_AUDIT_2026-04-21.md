# Creem Flow Security Audit

Date: 2026-04-21
Scope: Creem checkout, webhook ingress, normalization/processing, and related DB mutation paths.

## Findings (ordered by severity)

2. **Webhook route token is logged in plaintext.**
- The Creem webhook endpoint token is written to logs, reducing endpoint secrecy if logs are exposed.
- Reference:
  - `src/webhooks/ingress.rs:229`

3. **Outbound Creem HTTP client has no explicit timeout.**
- `reqwest::Client::new()` is used without timeout configuration, which can allow slow/hanging upstream requests to consume worker capacity.
- Reference:
  - `src/services/creem/client.rs:19`

4. **Creem API error bodies are logged directly.**
- Provider response bodies are logged on failures; these may contain sensitive operational or customer data.
- References:
  - `src/services/creem/client.rs:113`
  - `src/services/creem/client.rs:142`
  - `src/services/creem/client.rs:173`
  - `src/services/creem/client.rs:210`

5. **Dependency-level vulnerabilities affect the Creem outbound TLS path.**
- `cargo audit` reported:
  - `RUSTSEC-2026-0098` (`rustls-webpki`)
  - `RUSTSEC-2026-0099` (`rustls-webpki`)
- These are in the `reqwest` TLS dependency chain used for Creem API calls.
- Suggested direction: upgrade to `rustls-webpki >= 0.103.12` via compatible dependency updates.

## Audit Notes

- No code changes were made during the audit.
- This document captures findings only.
