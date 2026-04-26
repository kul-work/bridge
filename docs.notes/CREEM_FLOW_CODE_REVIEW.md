# Creem Flow Code Review

Date: 2026-04-24

Scope: current `main` branch review of the Creem checkout, webhook ingress, webhook processing, subscription action, forwarding, and retry flow.

## Findings

### Medium: short or malformed Creem webhook tokens can panic before validation

`handle_creem` logs `&token[..8]` before parsing the token as a UUID. A request such as `/webhooks/a/creem` can panic from slicing a short string instead of returning `404`.

Relevant code:

- `src/webhooks/ingress.rs`: `info!("Received Creem webhook with token: {}...", &token[..8]);`

Impact: malformed public webhook requests can crash the request task and produce noisy operational failures.

Recommendation: parse the UUID first, or log with `token.chars().take(8).collect::<String>()`.

### Medium: invalid cancellation modes are sent to Creem before local validation

`cancel_subscription` defaults or reads `mode`, sends it to the provider API, and only validates it against `"scheduled"` and `"immediate"` afterward. For Creem, this can emit an unsupported or unintended API request before Bridge rejects the request locally.

Relevant code:

- `src/application/subscription_actions.rs`: `mode` is passed into `provider_api::cancel_subscription`
- `src/application/subscription_actions.rs`: validation happens later in the local DB update `match`

Impact: bad client input can leak into the external provider call and cause inconsistent behavior between Creem and Bridge.

Recommendation: validate `mode` immediately after reading it and before any provider call.
