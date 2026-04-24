# Creem Flow Code Review

Date: 2026-04-24

Scope: current `main` branch review of the Creem checkout, webhook ingress, webhook processing, subscription action, forwarding, and retry flow.

## Findings

### High: Creem webhooks can be acknowledged and then permanently stranded

`handle_creem` inserts a `pay.webhook_provider` row, spawns processing in the background, and immediately returns `204`. If `process_webhook` fails before a `webhook_delivery` row is created, the error is only logged. A later Creem retry with the same event ID hits the duplicate path and returns `204` without retrying processing.

Relevant code:

- `src/webhooks/ingress.rs`: `spawn_process_and_forward_webhook`
- `src/webhooks/ingress.rs`: `handle_creem` duplicate path after `create_webhook_provider`
- `src/webhooks/scheduler.rs`: retry worker only retries pending `webhook_delivery` rows

Impact: a valid paid webhook can be stored but never mutate Bridge state and never forward to the app. Creem will believe delivery succeeded because Bridge already returned a 2xx response.

Recommendation: process synchronously before returning 2xx, or make duplicate/unprocessed provider rows resumable. At minimum, duplicates with `processed = false` should re-enter processing instead of returning success immediately.

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

### Medium: Creem checkout ignores most requested product IDs when choosing the provider product

For Creem, `product_selector` is derived from `product_type.unwrap_or(product_id)`, but unless the selector is exactly `"offer"` or `"otp"`, Bridge sends `creem_config.product_id` to Creem. The requested API `product_id` is only placed in metadata.

Relevant code:

- `src/application/checkout.rs`: `product_selector` and `selected_product_id` logic

Impact: apps with multiple Creem products can request one product while Bridge creates a checkout for the configured default product.

Recommendation: either map requested Bridge product IDs to Creem product IDs explicitly in provider config, or reject unsupported product IDs instead of silently falling back to the default.
