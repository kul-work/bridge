#!/bin/bash

##############################################################################
# CROSS-APP-READ: Cross-App Tenant Isolation Security Test
#
# Purpose: Verify that App B's API key cannot read or mutate App A's payment
#          and subscription data. Bridge's sole defense is app_id-scoped
#          queries + PostgreSQL RLS (set_local_app_id + tenant_isolation_*
#          policies). This test exercises the real API path end-to-end.
#
# Design: App A (hiha) is seeded with a subscription + payment for a disposable
#         test user via direct DB insert (bridge_admin bypasses RLS). App B
#         (household) then attempts to read that user's data through every
#         payment-facing read endpoint and one write endpoint (cancel).
#         Every attempt must return empty results or 404 — never App A's data.
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

# Resolve app IDs and validate API keys are set.
APP_A_ID=$(psql_exec "SELECT id FROM pay.apps WHERE slug = $(sql_literal "$BRIDGE_APP_A_SLUG");")
APP_B_ID=$(psql_exec "SELECT id FROM pay.apps WHERE slug = $(sql_literal "$BRIDGE_APP_B_SLUG");")

[[ -n "$APP_A_ID" ]] || fail "App A ('$BRIDGE_APP_A_SLUG') not found in pay.apps"
[[ -n "$APP_B_ID" ]] || fail "App B ('$BRIDGE_APP_B_SLUG') not found in pay.apps"
[[ -n "${BRIDGE_APP_A_API_KEY:-}" ]] || fail "BRIDGE_APP_A_API_KEY not set (check tests/security/.env)"
[[ -n "${BRIDGE_APP_B_API_KEY:-}" ]] || fail "BRIDGE_APP_B_API_KEY not set (check tests/security/.env)"

USER_LITERAL=$(sql_literal "$TEST_USER_ID")
SUB_LITERAL=$(sql_literal "$SUBSCRIPTION_ID")
TOKEN_LITERAL=$(sql_literal "$PURCHASE_TOKEN")
PAY_TXN_LITERAL=$(sql_literal "$PAY_TXN_ID")
APP_A_LITERAL=$(sql_literal "$APP_A_ID")

cleanup() {
    PGPASSWORD="$PGPASSWORD" psql \
        -U "$BRIDGE_DB_USER" \
        -h "$BRIDGE_DB_HOST" \
        -p "$BRIDGE_DB_PORT" \
        -d "$BRIDGE_DB_NAME" \
        -c "DELETE FROM pay.payments WHERE external_user_id = $USER_LITERAL; DELETE FROM pay.subscriptions WHERE external_user_id = $USER_LITERAL;" >/dev/null 2>&1 || true
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
# Setup: seed App A with a subscription + payment for the test user
##############################################################################
echo -e "${YELLOW}[1/7] Seed App A with subscription + payment${NC}"

psql_exec "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, purchase_token, status, current_period_end, auto_renewing, payment_state) VALUES ($APP_A_LITERAL, $USER_LITERAL, $SUB_LITERAL, 'google_play', $TOKEN_LITERAL, 'active', NOW() + INTERVAL '30 days', true, 1);" >/dev/null

psql_exec "INSERT INTO pay.payments (app_id, external_user_id, provider, provider_transaction_id, subscription_id, product_id, amount_cents, currency, status) VALUES ($APP_A_LITERAL, $USER_LITERAL, 'google_play', $PAY_TXN_LITERAL, $SUB_LITERAL, 'hiha_monthly', 499, 'USD', 'success');" >/dev/null

echo -e "${GREEN}✓ App A seeded (sub=$SUBSCRIPTION_ID, payment=$PAY_TXN_ID)${NC}"
echo ""

##############################################################################
# Sanity: App A can read its own data
##############################################################################
echo -e "${YELLOW}[2/7] Sanity: App A reads own data${NC}"

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
echo ""

##############################################################################
# Isolation 1: App B GET /payments — must see empty
##############################################################################
echo -e "${YELLOW}[3/7] App B GET /payments isolation${NC}"

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
echo -e "${YELLOW}[4/7] App B GET /subscriptions isolation${NC}"

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
# Isolation 3: App B GET /users/:id/subscription-status — must see is_premium=false
##############################################################################
echo -e "${YELLOW}[5/7] App B GET /users/:id/subscription-status isolation${NC}"

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
# Isolation 4: App B GET /users/:id/data-export — must see empty arrays
##############################################################################
echo -e "${YELLOW}[6/7] App B GET /users/:id/data-export isolation${NC}"

EXPORT_B=$(bridge_request GET "/api/v1/users/$TEST_USER_ID/data-export" "$BRIDGE_APP_B_API_KEY")
assert_http_code "$(response_code "$EXPORT_B")" "200" "$(response_body "$EXPORT_B")" "App B data-export"
EXPORT_B_SUBS=$(printf "%s" "$(response_body "$EXPORT_B")" | jq -r '.subscriptions | length')
EXPORT_B_PAYS=$(printf "%s" "$(response_body "$EXPORT_B")" | jq -r '.payments | length')
if [[ "$EXPORT_B_SUBS" != "0" ]]; then
    fail "App B data-export leak: $EXPORT_B_SUBS subscription(s) in export, expected 0"
fi
if [[ "$EXPORT_B_PAYS" != "0" ]]; then
    fail "App B data-export leak: $EXPORT_B_PAYS payment(s) in export, expected 0"
fi
if printf "%s" "$(response_body "$EXPORT_B")" | grep -q "$PAY_TXN_ID"; then
    fail "App B data-export leak: App A payment marker found in export"
fi
if printf "%s" "$(response_body "$EXPORT_B")" | grep -q "$SUBSCRIPTION_ID"; then
    fail "App B data-export leak: App A subscription_id found in export"
fi
echo -e "${GREEN}✓ App B data-export has 0 subscriptions, 0 payments (no leak)${NC}"
echo ""

##############################################################################
# Isolation 5: App B POST /subscriptions/:id/cancel — must get 404
##############################################################################
echo -e "${YELLOW}[7/7] App B POST /subscriptions/:id/cancel isolation${NC}"

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
# Cleanup and verify
##############################################################################
cleanup
trap - EXIT

LEFTOVER_SUBS=$(psql_exec "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = $USER_LITERAL;")
LEFTOVER_PAYS=$(psql_exec "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = $USER_LITERAL;")
if [[ "$LEFTOVER_SUBS" != "0" || "$LEFTOVER_PAYS" != "0" ]]; then
    fail "Cleanup incomplete: subs=$LEFTOVER_SUBS pays=$LEFTOVER_PAYS"
fi
echo -e "${GREEN}✓ Cleanup verified (0 subs, 0 payments remaining)${NC}"
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
    "app_a_sanity_payments": "pass",
    "app_a_sanity_subscriptions": "pass",
    "app_b_payments_isolated": "pass",
    "app_b_subscriptions_isolated": "pass",
    "app_b_subscription_status_isolated": "pass",
    "app_b_data_export_isolated": "pass",
    "app_b_cancel_rejected": "pass",
    "cleanup_complete": "pass"
  }
}
EOF

echo -e "${GREEN}✓ CROSS-APP-READ security test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
