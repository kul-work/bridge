#!/bin/bash

##############################################################################
# CROSS-APP-READ: Cross-App Tenant Isolation Security Test
#
# Purpose: Verify that App B's API key cannot read or mutate App A's payment
#          and subscription data. Bridge's sole defense is app_id-scoped
#          queries + PostgreSQL RLS (set_local_app_id + tenant_isolation_*
#          policies). This test exercises the real API path end-to-end.
#
# Design: App A (hiha) is seeded with a subscription + payment + webhook
#         record for a disposable test user via direct DB insert (bridge_admin
#         bypasses RLS). App B (household) then attempts to read that user's
#         data through every payment-facing read endpoint and one write
#         endpoint (cancel). Every attempt must return empty results or 404 —
#         never App A's data. After the cancel attempt, App A's subscription
#         is re-verified durably to confirm no data corruption.
#
# Kind: Shell security test for a running backend, not an in-process Rust
#       integration test.
#
# Usage: ./test-cross-app-read.sh
#
# Prerequisites:
#   - Bridge backend is already running at BRIDGE_API_URL.
#   - curl, jq, and psql are installed and in PATH.
#   - tests/security/globals.cfg is configured, optionally via
#     tests/security/.env (must contain BRIDGE_APP_A_API_KEY and
#     BRIDGE_APP_B_API_KEY).
##############################################################################

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

TEST_RUN_ID="sec-xapp-$(date +%s)-$$"
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
REPORT_FILE="$SCRIPT_DIR/cross-app-read-report.json"

TEST_USER_ID="security_alice_xapp_${TEST_RUN_ID}"
SUBSCRIPTION_ID="sec-sub-${TEST_RUN_ID}"
PURCHASE_TOKEN="sec-token-${TEST_RUN_ID}"
PAY_TXN_ID="sec-pay-txn-${TEST_RUN_ID}"
WEBHOOK_ID="sec-whk-${TEST_RUN_ID}"

require_command() {
    local command_name="$1"
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Missing required command: $command_name" >&2
        exit 1
    fi
}

response_code() {
    printf "%s" "$1" | tail -n1
}

response_body() {
    printf "%s" "$1" | sed '$d'
}

assert_http_code() {
    local actual="$1"
    local expected="$2"
    local body="$3"
    local label="$4"

    if [[ "$actual" != "$expected" ]]; then
        echo -e "${RED}✗ $label failed: HTTP $actual, expected $expected${NC}" >&2
        echo "$body" >&2
        exit 1
    fi
}

sql_literal() {
    local escaped
    escaped=$(printf "%s" "$1" | sed "s/'/''/g")
    printf "'%s'" "$escaped"
}

psql_exec() {
    PGPASSWORD="$PGPASSWORD" psql \
        -U "$BRIDGE_DB_USER" \
        -h "$BRIDGE_DB_HOST" \
        -p "$BRIDGE_DB_PORT" \
        -d "$BRIDGE_DB_NAME" \
        -At \
        -c "$1"
}

# bridge_request <method> <path> <api_key> [body]
bridge_request() {
    local method="$1"
    local path="$2"
    local api_key="$3"
    local body="${4:-}"

    if [[ -n "$body" ]]; then
        curl -sS -w "\n%{http_code}" -X "$method" \
            "$BRIDGE_API_URL$path" \
            -H "Content-Type: application/json" \
            -H "Authorization: Bearer $api_key" \
            -d "$body"
    else
        curl -sS -w "\n%{http_code}" -X "$method" \
            "$BRIDGE_API_URL$path" \
            -H "Content-Type: application/json" \
            -H "Authorization: Bearer $api_key"
    fi
}

fail() {
    echo -e "${RED}✗ $1${NC}" >&2
    exit 1
}

require_command curl
require_command jq
require_command psql

BRIDGE_API_URL="${BRIDGE_API_URL%/}"

# Resolve app IDs from slugs.
APP_A_ID=$(psql_exec "SELECT id FROM pay.apps WHERE slug = $(sql_literal "$BRIDGE_APP_A_SLUG");")
APP_B_ID=$(psql_exec "SELECT id FROM pay.apps WHERE slug = $(sql_literal "$BRIDGE_APP_B_SLUG");")

[[ -n "$APP_A_ID" ]] || fail "App A ('$BRIDGE_APP_A_SLUG') not found in pay.apps"
[[ -n "$APP_B_ID" ]] || fail "App B ('$BRIDGE_APP_B_SLUG') not found in pay.apps"
[[ -n "${BRIDGE_APP_A_API_KEY:-}" ]] || fail "BRIDGE_APP_A_API_KEY not set (check tests/security/.env)"
[[ -n "${BRIDGE_APP_B_API_KEY:-}" ]] || fail "BRIDGE_APP_B_API_KEY not set (check tests/security/.env)"

# Prove each API key actually belongs to the expected app. The key prefix
# (first 8 chars) is stored in pay.api_keys and maps to an app_id. Without
# this check, a swapped key would make the test pass vacuously.
APP_A_KEY_PREFIX="${BRIDGE_APP_A_API_KEY:0:8}"
APP_B_KEY_PREFIX="${BRIDGE_APP_B_API_KEY:0:8}"
KEY_A_APP_ID=$(psql_exec "SELECT app_id FROM pay.api_keys WHERE key_prefix = $(sql_literal "$APP_A_KEY_PREFIX") LIMIT 1;")
KEY_B_APP_ID=$(psql_exec "SELECT app_id FROM pay.api_keys WHERE key_prefix = $(sql_literal "$APP_B_KEY_PREFIX") LIMIT 1;")

[[ "$KEY_A_APP_ID" == "$APP_A_ID" ]] || fail "API key A prefix '$APP_A_KEY_PREFIX' maps to app $KEY_A_APP_ID, expected $APP_A_ID ($BRIDGE_APP_A_SLUG). Check .env keys."
[[ "$KEY_B_APP_ID" == "$APP_B_ID" ]] || fail "API key B prefix '$APP_B_KEY_PREFIX' maps to app $KEY_B_APP_ID, expected $APP_B_ID ($BRIDGE_APP_B_SLUG). Check .env keys."

USER_LITERAL=$(sql_literal "$TEST_USER_ID")
SUB_LITERAL=$(sql_literal "$SUBSCRIPTION_ID")
TOKEN_LITERAL=$(sql_literal "$PURCHASE_TOKEN")
PAY_TXN_LITERAL=$(sql_literal "$PAY_TXN_ID")
WEBHOOK_LITERAL=$(sql_literal "$WEBHOOK_ID")
APP_A_LITERAL=$(sql_literal "$APP_A_ID")

cleanup() {
    PGPASSWORD="$PGPASSWORD" psql \
        -U "$BRIDGE_DB_USER" \
        -h "$BRIDGE_DB_HOST" \
        -p "$BRIDGE_DB_PORT" \
        -d "$BRIDGE_DB_NAME" \
        -c "DELETE FROM pay.webhook_provider WHERE app_id = $APP_A_LITERAL AND provider_webhook_id = $WEBHOOK_LITERAL; DELETE FROM pay.payments WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL; DELETE FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL;" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo -e "${YELLOW}========================================${NC}"
echo "CROSS-APP-READ: Cross-App Tenant Isolation"
echo -e "${YELLOW}========================================${NC}"
echo "Test Run ID: $TEST_RUN_ID"
echo "App A (owner):   $BRIDGE_APP_A_SLUG ($APP_A_ID)"
echo "App B (attacker): $BRIDGE_APP_B_SLUG ($APP_B_ID)"
echo "Test user:       $TEST_USER_ID"
echo ""

##############################################################################
# Setup: seed App A with a subscription + payment + webhook for the test user
##############################################################################
echo -e "${YELLOW}[1/10] Seed App A with subscription + payment + webhook${NC}"

psql_exec "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, purchase_token, status, current_period_end, auto_renewing, payment_state) VALUES ($APP_A_LITERAL, $USER_LITERAL, $SUB_LITERAL, 'google_play', $TOKEN_LITERAL, 'active', NOW() + INTERVAL '30 days', true, 1);" >/dev/null

psql_exec "INSERT INTO pay.payments (app_id, external_user_id, provider, provider_transaction_id, subscription_id, product_id, amount_cents, currency, status) VALUES ($APP_A_LITERAL, $USER_LITERAL, 'google_play', $PAY_TXN_LITERAL, $SUB_LITERAL, 'hiha_monthly', 499, 'USD', 'success');" >/dev/null

psql_exec "INSERT INTO pay.webhook_provider (app_id, provider, provider_webhook_id, event_type, subscription_id, purchase_token, payload) VALUES ($APP_A_LITERAL, 'google_play', $WEBHOOK_LITERAL, 'SUBSCRIPTION_PURCHASED', $SUB_LITERAL, $TOKEN_LITERAL, '{\"test_marker\": \"$WEBHOOK_ID\"}'::jsonb);" >/dev/null

echo -e "${GREEN}✓ App A seeded (sub=$SUBSCRIPTION_ID, payment=$PAY_TXN_ID, webhook=$WEBHOOK_ID)${NC}"
echo ""

##############################################################################
# Sanity: App A can read its own data (payments, subscriptions, detail, export)
##############################################################################
echo -e "${YELLOW}[2/10] Sanity: App A reads own data${NC}"

PAY_A=$(bridge_request GET "/api/v1/payments?external_user_id=$TEST_USER_ID" "$BRIDGE_APP_A_API_KEY")
assert_http_code "$(response_code "$PAY_A")" "200" "$(response_body "$PAY_A")" "App A payments"
PAY_A_TOTAL=$(printf "%s" "$(response_body "$PAY_A")" | jq -r '.total')
PAY_A_COUNT=$(printf "%s" "$(response_body "$PAY_A")" | jq -r '.payments | length')
[[ "$PAY_A_TOTAL" -ge 1 ]] || fail "App A sanity: expected total >= 1, got $PAY_A_TOTAL"
[[ "$PAY_A_COUNT" -ge 1 ]] || fail "App A sanity: expected >= 1 payment, got $PAY_A_COUNT"
echo -e "${GREEN}✓ App A sees $PAY_A_COUNT payment(s), total=$PAY_A_TOTAL${NC}"

SUB_A=$(bridge_request GET "/api/v1/subscriptions?external_user_id=$TEST_USER_ID" "$BRIDGE_APP_A_API_KEY")
assert_http_code "$(response_code "$SUB_A")" "200" "$(response_body "$SUB_A")" "App A subscriptions"
SUB_A_COUNT=$(printf "%s" "$(response_body "$SUB_A")" | jq -r '.subscriptions | length')
[[ "$SUB_A_COUNT" -ge 1 ]] || fail "App A sanity: expected >= 1 subscription, got $SUB_A_COUNT"
echo -e "${GREEN}✓ App A sees $SUB_A_COUNT subscription(s)${NC}"

SUB_DETAIL_A=$(bridge_request GET "/api/v1/subscriptions/$SUBSCRIPTION_ID?external_user_id=$TEST_USER_ID&provider=google_play" "$BRIDGE_APP_A_API_KEY")
assert_http_code "$(response_code "$SUB_DETAIL_A")" "200" "$(response_body "$SUB_DETAIL_A")" "App A subscription detail"
SUB_DETAIL_A_ID=$(printf "%s" "$(response_body "$SUB_DETAIL_A")" | jq -r '.subscription_id')
[[ "$SUB_DETAIL_A_ID" == "$SUBSCRIPTION_ID" ]] || fail "App A subscription detail: expected subscription_id=$SUBSCRIPTION_ID, got $SUB_DETAIL_A_ID"
echo -e "${GREEN}✓ App A sees subscription detail (id=$SUB_DETAIL_A_ID)${NC}"

EXPORT_A=$(bridge_request GET "/api/v1/users/$TEST_USER_ID/data-export" "$BRIDGE_APP_A_API_KEY")
assert_http_code "$(response_code "$EXPORT_A")" "200" "$(response_body "$EXPORT_A")" "App A data-export"
EXPORT_A_WHKS=$(printf "%s" "$(response_body "$EXPORT_A")" | jq -r '.webhook_records | length')
[[ "$EXPORT_A_WHKS" -ge 1 ]] || fail "App A data-export sanity: expected >= 1 webhook record, got $EXPORT_A_WHKS"
if ! printf "%s" "$(response_body "$EXPORT_A")" | grep -q "$WEBHOOK_ID"; then
    fail "App A data-export sanity: webhook marker $WEBHOOK_ID not found in export"
fi
echo -e "${GREEN}✓ App A data-export has $EXPORT_A_WHKS webhook record(s)${NC}"
echo ""

##############################################################################
# Isolation 1: App B GET /payments — must see empty
##############################################################################
echo -e "${YELLOW}[3/10] App B GET /payments isolation${NC}"

PAY_B=$(bridge_request GET "/api/v1/payments?external_user_id=$TEST_USER_ID" "$BRIDGE_APP_B_API_KEY")
assert_http_code "$(response_code "$PAY_B")" "200" "$(response_body "$PAY_B")" "App B payments"
PAY_B_TOTAL=$(printf "%s" "$(response_body "$PAY_B")" | jq -r '.total')
PAY_B_COUNT=$(printf "%s" "$(response_body "$PAY_B")" | jq -r '.payments | length')
if [[ "$PAY_B_TOTAL" != "0" ]]; then
    fail "App B payments leak: total=$PAY_B_TOTAL, expected 0"
fi
if [[ "$PAY_B_COUNT" != "0" ]]; then
    fail "App B payments leak: $PAY_B_COUNT payment(s) returned, expected 0"
fi
if printf "%s" "$(response_body "$PAY_B")" | grep -q "$PAY_TXN_ID"; then
    fail "App B payments leak: App A payment marker $PAY_TXN_ID found in response"
fi
echo -e "${GREEN}✓ App B sees 0 payments (no leak)${NC}"
echo ""

##############################################################################
# Isolation 2: App B GET /subscriptions — must see empty
##############################################################################
echo -e "${YELLOW}[4/10] App B GET /subscriptions isolation${NC}"

SUB_B=$(bridge_request GET "/api/v1/subscriptions?external_user_id=$TEST_USER_ID" "$BRIDGE_APP_B_API_KEY")
assert_http_code "$(response_code "$SUB_B")" "200" "$(response_body "$SUB_B")" "App B subscriptions"
SUB_B_COUNT=$(printf "%s" "$(response_body "$SUB_B")" | jq -r '.subscriptions | length')
if [[ "$SUB_B_COUNT" != "0" ]]; then
    fail "App B subscriptions leak: $SUB_B_COUNT subscription(s) returned, expected 0"
fi
if printf "%s" "$(response_body "$SUB_B")" | grep -q "$SUBSCRIPTION_ID"; then
    fail "App B subscriptions leak: App A subscription_id $SUBSCRIPTION_ID found in response"
fi
echo -e "${GREEN}✓ App B sees 0 subscriptions (no leak)${NC}"
echo ""

##############################################################################
# Isolation 3: App B GET /subscriptions/:id — must get 404
##############################################################################
echo -e "${YELLOW}[5/10] App B GET /subscriptions/:id isolation${NC}"

SUB_DETAIL_B=$(bridge_request GET "/api/v1/subscriptions/$SUBSCRIPTION_ID?external_user_id=$TEST_USER_ID&provider=google_play" "$BRIDGE_APP_B_API_KEY")
SUB_DETAIL_B_CODE=$(response_code "$SUB_DETAIL_B")
SUB_DETAIL_B_BODY=$(response_body "$SUB_DETAIL_B")
if [[ "$SUB_DETAIL_B_CODE" != "404" ]]; then
    fail "App B subscription detail leak: HTTP $SUB_DETAIL_B_CODE, expected 404. Body: $SUB_DETAIL_B_BODY"
fi
if ! printf "%s" "$SUB_DETAIL_B_BODY" | grep -q "subscription_not_found"; then
    fail "App B subscription detail: expected error code 'subscription_not_found', got: $SUB_DETAIL_B_BODY"
fi
echo -e "${GREEN}✓ App B subscription detail rejected with 404 subscription_not_found${NC}"
echo ""

##############################################################################
# Isolation 4: App B GET /users/:id/subscription-status — must see is_premium=false
##############################################################################
echo -e "${YELLOW}[6/10] App B GET /users/:id/subscription-status isolation${NC}"

STATUS_B=$(bridge_request GET "/api/v1/users/$TEST_USER_ID/subscription-status" "$BRIDGE_APP_B_API_KEY")
assert_http_code "$(response_code "$STATUS_B")" "200" "$(response_body "$STATUS_B")" "App B subscription-status"
STATUS_B_PREMIUM=$(printf "%s" "$(response_body "$STATUS_B")" | jq -r '.is_premium')
STATUS_B_SUB_ID=$(printf "%s" "$(response_body "$STATUS_B")" | jq -r '.subscription_id')
if [[ "$STATUS_B_PREMIUM" == "true" ]]; then
    fail "App B subscription-status leak: is_premium=true, expected false (subscription belongs to App A)"
fi
if [[ "$STATUS_B_SUB_ID" != "null" ]]; then
    fail "App B subscription-status leak: subscription_id=$STATUS_B_SUB_ID, expected null"
fi
echo -e "${GREEN}✓ App B sees is_premium=false, subscription_id=null (no leak)${NC}"
echo ""

##############################################################################
# Isolation 5: App B GET /users/:id/data-export — must see empty arrays
##############################################################################
echo -e "${YELLOW}[7/10] App B GET /users/:id/data-export isolation${NC}"

EXPORT_B=$(bridge_request GET "/api/v1/users/$TEST_USER_ID/data-export" "$BRIDGE_APP_B_API_KEY")
assert_http_code "$(response_code "$EXPORT_B")" "200" "$(response_body "$EXPORT_B")" "App B data-export"
EXPORT_B_SUBS=$(printf "%s" "$(response_body "$EXPORT_B")" | jq -r '.subscriptions | length')
EXPORT_B_PAYS=$(printf "%s" "$(response_body "$EXPORT_B")" | jq -r '.payments | length')
EXPORT_B_WHKS=$(printf "%s" "$(response_body "$EXPORT_B")" | jq -r '.webhook_records | length')
if [[ "$EXPORT_B_SUBS" != "0" ]]; then
    fail "App B data-export leak: $EXPORT_B_SUBS subscription(s) in export, expected 0"
fi
if [[ "$EXPORT_B_PAYS" != "0" ]]; then
    fail "App B data-export leak: $EXPORT_B_PAYS payment(s) in export, expected 0"
fi
if [[ "$EXPORT_B_WHKS" != "0" ]]; then
    fail "App B data-export leak: $EXPORT_B_WHKS webhook record(s) in export, expected 0"
fi
if printf "%s" "$(response_body "$EXPORT_B")" | grep -q "$PAY_TXN_ID"; then
    fail "App B data-export leak: App A payment marker found in export"
fi
if printf "%s" "$(response_body "$EXPORT_B")" | grep -q "$SUBSCRIPTION_ID"; then
    fail "App B data-export leak: App A subscription_id found in export"
fi
if printf "%s" "$(response_body "$EXPORT_B")" | grep -q "$WEBHOOK_ID"; then
    fail "App B data-export leak: App A webhook marker found in export"
fi
echo -e "${GREEN}✓ App B data-export has 0 subscriptions, 0 payments, 0 webhook records (no leak)${NC}"
echo ""

##############################################################################
# Isolation 6: App B POST /subscriptions/:id/cancel — must get 404
##############################################################################
echo -e "${YELLOW}[8/10] App B POST /subscriptions/:id/cancel isolation${NC}"

# Capture App A's subscription version before the cancel attempt.
SUB_VERSION_BEFORE=$(psql_exec "SELECT version FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL AND subscription_id = $SUB_LITERAL AND provider = 'google_play';")

CANCEL_B=$(bridge_request POST "/api/v1/subscriptions/$SUBSCRIPTION_ID/cancel?external_user_id=$TEST_USER_ID&provider=google_play" "$BRIDGE_APP_B_API_KEY")
CANCEL_B_CODE=$(response_code "$CANCEL_B")
CANCEL_B_BODY=$(response_body "$CANCEL_B")
if [[ "$CANCEL_B_CODE" != "404" ]]; then
    fail "App B cancel leak: HTTP $CANCEL_B_CODE, expected 404. Body: $CANCEL_B_BODY"
fi
if ! printf "%s" "$CANCEL_B_BODY" | grep -q "subscription_not_found"; then
    fail "App B cancel: expected error code 'subscription_not_found', got: $CANCEL_B_BODY"
fi
echo -e "${GREEN}✓ App B cancel rejected with 404 subscription_not_found${NC}"
echo ""

##############################################################################
# Durable verification: App A's subscription must be untouched after cancel
##############################################################################
echo -e "${YELLOW}[9/10] Durable: App A subscription intact after App B cancel${NC}"

SUB_AFTER_STATUS=$(psql_exec "SELECT status FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL AND subscription_id = $SUB_LITERAL AND provider = 'google_play';")
SUB_AFTER_AUTO_RENEW=$(psql_exec "SELECT auto_renewing FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL AND subscription_id = $SUB_LITERAL AND provider = 'google_play';")
SUB_AFTER_CANCEL_INIT=$(psql_exec "SELECT cancellation_initiated_at IS NULL FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL AND subscription_id = $SUB_LITERAL AND provider = 'google_play';")
SUB_AFTER_REVOKED=$(psql_exec "SELECT revoked_at IS NULL FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL AND subscription_id = $SUB_LITERAL AND provider = 'google_play';")
SUB_AFTER_VERSION=$(psql_exec "SELECT version FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL AND subscription_id = $SUB_LITERAL AND provider = 'google_play';")

[[ "$SUB_AFTER_STATUS" == "active" ]] || fail "App A subscription corrupted: status=$SUB_AFTER_STATUS, expected active"
[[ "$SUB_AFTER_AUTO_RENEW" == "t" ]] || fail "App A subscription corrupted: auto_renewing=$SUB_AFTER_AUTO_RENEW, expected true"
[[ "$SUB_AFTER_CANCEL_INIT" == "t" ]] || fail "App A subscription corrupted: cancellation_initiated_at is not NULL"
[[ "$SUB_AFTER_REVOKED" == "t" ]] || fail "App A subscription corrupted: revoked_at is not NULL"
[[ "$SUB_AFTER_VERSION" == "$SUB_VERSION_BEFORE" ]] || fail "App A subscription corrupted: version changed from $SUB_VERSION_BEFORE to $SUB_AFTER_VERSION"
echo -e "${GREEN}✓ App A subscription intact (status=active, auto_renewing=true, no cancel, no revoke, version unchanged)${NC}"

# Re-read via App A's API key to confirm the subscription is still visible and active.
SUB_REREAD_A=$(bridge_request GET "/api/v1/subscriptions/$SUBSCRIPTION_ID?external_user_id=$TEST_USER_ID&provider=google_play" "$BRIDGE_APP_A_API_KEY")
assert_http_code "$(response_code "$SUB_REREAD_A")" "200" "$(response_body "$SUB_REREAD_A")" "App A re-read after cancel"
SUB_REREAD_STATUS=$(printf "%s" "$(response_body "$SUB_REREAD_A")" | jq -r '.status')
[[ "$SUB_REREAD_STATUS" == "active" ]] || fail "App A re-read: status=$SUB_REREAD_STATUS, expected active"
echo -e "${GREEN}✓ App A re-reads subscription as active via API${NC}"
echo ""

##############################################################################
# Cleanup and verify
##############################################################################
echo -e "${YELLOW}[10/10] Cleanup and verify${NC}"
cleanup
trap - EXIT

LEFTOVER_SUBS=$(psql_exec "SELECT COUNT(*) FROM pay.subscriptions WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL;")
LEFTOVER_PAYS=$(psql_exec "SELECT COUNT(*) FROM pay.payments WHERE app_id = $APP_A_LITERAL AND external_user_id = $USER_LITERAL;")
LEFTOVER_WHKS=$(psql_exec "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = $APP_A_LITERAL AND provider_webhook_id = $WEBHOOK_LITERAL;")
if [[ "$LEFTOVER_SUBS" != "0" || "$LEFTOVER_PAYS" != "0" || "$LEFTOVER_WHKS" != "0" ]]; then
    fail "Cleanup incomplete: subs=$LEFTOVER_SUBS pays=$LEFTOVER_PAYS whks=$LEFTOVER_WHKS"
fi
echo -e "${GREEN}✓ Cleanup verified (0 subs, 0 payments, 0 webhooks remaining)${NC}"
echo ""

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "CROSS-APP-READ",
  "test_name": "Cross-App Tenant Isolation",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "app_a_slug": "$BRIDGE_APP_A_SLUG",
  "app_a_id": "$APP_A_ID",
  "app_b_slug": "$BRIDGE_APP_B_SLUG",
  "app_b_id": "$APP_B_ID",
  "test_user_id": "$TEST_USER_ID",
  "checks": {
    "api_key_a_identity_verified": "pass",
    "api_key_b_identity_verified": "pass",
    "app_a_sanity_payments": "pass",
    "app_a_sanity_subscriptions": "pass",
    "app_a_sanity_subscription_detail": "pass",
    "app_a_sanity_data_export_webhooks": "pass",
    "app_b_payments_isolated": "pass",
    "app_b_subscriptions_isolated": "pass",
    "app_b_subscription_detail_isolated": "pass",
    "app_b_subscription_status_isolated": "pass",
    "app_b_data_export_isolated": "pass",
    "app_b_data_export_webhooks_isolated": "pass",
    "app_b_cancel_rejected": "pass",
    "app_a_subscription_intact_after_cancel": "pass",
    "app_a_subscription_reread_active": "pass",
    "cleanup_complete": "pass"
  }
}
EOF

echo -e "${GREEN}✓ CROSS-APP-READ security test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
