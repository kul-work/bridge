# BUG: pub_sub_audience ignored from DB config

`pay.provider_configs.config` may contain a `pub_sub_audience` key, but the
webhook ingress path reads it **only** from the ENV var
`GOOGLE_PUB_SUB_AUDIENCE` (`src/webhooks/ingress.rs:207`). The DB value is
never consulted, so per-app audience configuration is silently ignored.

Other Google settings in the same handler (`verify_webhook_signature`,
`service_account_json`) are correctly DB-driven, making this an inconsistency
rather than an intentional design.

**Impact:** Operators who set `pub_sub_audience` in the DB JSON expecting it to
apply will get silent fallback to the ENV var (or empty string if unset),
defeating per-app audience checks.

**Fix direction:** When `GOOGLE_PUB_SUB_AUDIENCE` is unset, fall back to
`provider_config.config["pub_sub_audience"]` before defaulting to empty.
