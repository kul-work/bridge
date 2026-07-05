#!/bin/bash

##############################################################################
# REN-01: Google Play Full Renewal Lifecycle Regression
#
# Purpose: Emulate the live Google Play test sequence:
#   1. SUBSCRIPTION_PURCHASED arrives before /verify-purchase and is suppressed.
#   2. The app calls /api/v1/verify-purchase.
#   3. Google sends repeated SUBSCRIPTION_RENEWED events with the same token.
#   4. Google sends SUBSCRIPTION_CANCELED and SUBSCRIPTION_EXPIRED.
#   5. Bridge and, when available, HiHa DB state are checked.
#
# Usage: ./test-ren-01.sh
#
# Prerequisites:
#   - Bridge running with MOCK_EXTERNAL_APIS=true.
#   - psql and curl installed.
#   - tests/gpbi/.env or environment provides BRIDGE_API_KEY.
#
# Notes:
#   - This script is intentionally standalone and is not included in
#     run-all-whk-tests.sh because it checks a currently known issue:
#     the initial verify-purchase payment should eventually use a provider
#     order id and non-null product_id, not purchase_token + NULL.
#   - In mock mode, renewal provider_transaction_id falls back to
#     google_play_rtdn:<message_id>. Live Google API enrichment should use GPA
#     order ids when latestOrderId is available.
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="ren-01-${TIMESTAMP}-$$"
REPORT_FILE="ren-01-report.json"

APP_ID="$BRIDGE_APP_ID"
APP_URL="${BRIDGE_API_URL:-http://localhost:5566}"
INGRESS_TOKEN="${WEBHOOK_INGRESS_TOKEN:-${WEBHOOK_TOKEN:-}}"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
USER_ID="${USER_ID:-test_ren_01_user_$TEST_RUN_ID}"
PURCHASE_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-ren-01-token-$TEST_RUN_ID"
PRICE_CENTS="${PRICE_CENTS:-549}"

if [[ -z "${BRIDGE_API_KEY:-}" ]]; then
    echo -e "${RED}[FAIL] BRIDGE_API_KEY is required${NC}"
    exit 1
fi

if [[ -z "$APP_ID" || -z "$INGRESS_TOKEN" ]]; then
    echo -e "${RED}[FAIL] BRIDGE_APP_ID and WEBHOOK_INGRESS_TOKEN/WEBHOOK_TOKEN are required${NC}"
    exit 1
fi

base64_no_wrap() {
    if base64 --help 2>&1 | grep -q -- "-w"; then
        base64 -w 0
    else
        base64 | tr -d '\n'
    fi
}

bridge_sql() {
    psql -v ON_ERROR_STOP=1 -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -At -c "$1"
}

HIHA_DB_MODE=""

detect_hiha_db_mode() {
    local bridge_schema_count
    bridge_schema_count=$(bridge_sql "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'hiha' AND table_name = 'webhook_callbacks';" 2>/dev/null | tr -d '[:space:]' || true)
    if [[ "$bridge_schema_count" == "1" ]]; then
        HIHA_DB_MODE="bridge_schema"
        return 0
    fi

    local separate_db_count
    separate_db_count=$(psql -v ON_ERROR_STOP=1 -U "$HIHA_DB_USER" -h "$HIHA_DB_HOST" -p "$HIHA_DB_PORT" -d "$HIHA_DB_NAME" -At \
        -c "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'webhook_callbacks';" 2>/dev/null | tr -d '[:space:]' || true)
    if [[ "$separate_db_count" == "1" ]]; then
        HIHA_DB_MODE="separate_public"
        return 0
    fi

    HIHA_DB_MODE=""
    return 1
}

hiha_sql() {
    local sql="$1"

    case "$HIHA_DB_MODE" in
        bridge_schema)
            psql -v ON_ERROR_STOP=1 -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -At -c "$sql"
            ;;
        separate_public)
            local public_sql
            public_sql="${sql//hiha.webhook_callbacks/webhook_callbacks}"
            public_sql="${public_sql//hiha.users/users}"
            psql -v ON_ERROR_STOP=1 -U "$HIHA_DB_USER" -h "$HIHA_DB_HOST" -p "$HIHA_DB_PORT" -d "$HIHA_DB_NAME" -At -c "$public_sql"
            ;;
        *)
            return 1
            ;;
    esac
}

wait_for_bridge_value() {
    local sql="$1"
    local expected="$2"
    local label="$3"
    local actual=""

    for _ in {1..20}; do
        actual="$(bridge_sql "$sql" 2>/dev/null | tr -d '[:space:]' || true)"
        if [[ "$actual" == "$expected" ]]; then
            echo -e "${GREEN}[OK] $label: $actual${NC}"
            return 0
        fi
        sleep 1
    done

    echo -e "${RED}[FAIL] $label: expected $expected, got ${actual:-<empty>}${NC}"
    return 1
}

send_google_subscription_webhook() {
    local notification_type="$1"
    local message_id="$2"
    local event_time_ms="$3"

    local notification_json
    notification_json=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$event_time_ms",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": $notification_type,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

    local notification_b64
    notification_b64=$(printf '%s' "$notification_json" | base64_no_wrap)

    local response
    response=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$INGRESS_TOKEN/google_play" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer test-token" \
        -H "X-Webhook-Verification-Mode: off" \
        -H "X-Test-Price-Cents: $PRICE_CENTS" \
        -d "{\"message\":{\"data\":\"$notification_b64\",\"message_id\":\"$message_id\",\"attributes\":{}},\"subscription\":\"projects/test-project/subscriptions/test-sub\"}")

    local http_code
    http_code=$(echo "$response" | tail -n1)
    if [[ "$http_code" != "200" && "$http_code" != "204" ]]; then
        echo -e "${RED}[FAIL] Webhook $message_id returned HTTP $http_code${NC}"
        echo "$response" | head -n -1
        return 1
    fi

    echo -e "${GREEN}[OK] Webhook $message_id accepted: HTTP $http_code${NC}"
}

echo -e "${YELLOW}========================================${NC}"
echo "REN-01: Google Play Full Renewal Lifecycle Regression"
echo -e "${YELLOW}========================================${NC}"
echo "Test Run ID: $TEST_RUN_ID"
echo "User ID:     $USER_ID"
echo "Token:       $PURCHASE_TOKEN"
echo ""

echo -e "${YELLOW}[1/7] Cleaning previous REN-01 data${NC}"
bridge_sql "DELETE FROM pay.webhook_delivery WHERE webhook_provider_id IN (SELECT id FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND (purchase_token LIKE 'test-ren-01-token-%' OR provider_webhook_id LIKE 'ren-01-%' OR payload->>'external_user_id' LIKE 'test_ren_01_user_%'));" >/dev/null
bridge_sql "DELETE FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND (purchase_token LIKE 'test-ren-01-token-%' OR provider_webhook_id LIKE 'ren-01-%' OR payload->>'external_user_id' LIKE 'test_ren_01_user_%');" >/dev/null
bridge_sql "DELETE FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND (external_user_id LIKE 'test_ren_01_user_%' OR provider_transaction_id LIKE 'test-ren-01-token-%' OR provider_transaction_id LIKE 'mock-google-play-order:test-ren-01-token-%' OR provider_transaction_id LIKE 'google_play_rtdn:ren-01-%');" >/dev/null
bridge_sql "DELETE FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id LIKE 'test_ren_01_user_%';" >/dev/null
detect_hiha_db_mode >/dev/null 2>&1 || true
hiha_sql "DELETE FROM hiha.webhook_callbacks WHERE clerk_id LIKE 'test_ren_01_user_%'; DELETE FROM hiha.users WHERE clerk_id LIKE 'test_ren_01_user_%';" >/dev/null || true
echo -e "${GREEN}[OK] Cleanup complete${NC}"
echo ""

BASE_EVENT_MS=$(($(date +%s) * 1000))
PURCHASED_ID="ren-01-purchased-$TEST_RUN_ID"
VERIFY_EVENT_ID=""
RENEWAL_ID_1="ren-01-renewal-1-$TEST_RUN_ID"
RENEWAL_ID_2="ren-01-renewal-2-$TEST_RUN_ID"
RENEWAL_ID_3="ren-01-renewal-3-$TEST_RUN_ID"
CANCELED_ID="ren-01-canceled-$TEST_RUN_ID"
EXPIRED_ID="ren-01-expired-$TEST_RUN_ID"

echo -e "${YELLOW}[2/7] Sending SUBSCRIPTION_PURCHASED before verify-purchase${NC}"
send_google_subscription_webhook 4 "$PURCHASED_ID" "$((BASE_EVENT_MS + 1000))"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_webhook_id = '$PURCHASED_ID' AND suppressed = true AND suppressed_reason = 'unresolved_external_user_id';" "1" "Purchased webhook suppressed before user resolution"
echo ""

echo -e "${YELLOW}[3/7] Registering purchase intent and calling /api/v1/verify-purchase${NC}"
REGISTER_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/api/v1/purchase/register" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $BRIDGE_API_KEY" \
    -d "{
      \"external_user_id\": \"$USER_ID\",
      \"provider\": \"$PROVIDER\",
      \"subscription_id\": \"$PRODUCT_ID\",
      \"reason\": \"ren-01-setup\"
    }")
REGISTER_HTTP=$(echo "$REGISTER_RESPONSE" | tail -n1)
if [[ "$REGISTER_HTTP" != "200" && "$REGISTER_HTTP" != "201" && "$REGISTER_HTTP" != "204" ]]; then
    echo -e "${RED}[FAIL] purchase/register returned HTTP $REGISTER_HTTP${NC}"
    echo "$REGISTER_RESPONSE" | head -n -1
    exit 1
fi
echo -e "${GREEN}[OK] purchase/register returned HTTP $REGISTER_HTTP${NC}"

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/api/v1/verify-purchase" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $BRIDGE_API_KEY" \
    -d "{
      \"external_user_id\": \"$USER_ID\",
      \"provider\": \"$PROVIDER\",
      \"subscription_id\": \"$PRODUCT_ID\",
      \"purchase_token\": \"$PURCHASE_TOKEN\",
      \"product_type\": \"subscription\"
    }")
VERIFY_HTTP=$(echo "$VERIFY_RESPONSE" | tail -n1)
if [[ "$VERIFY_HTTP" != "200" ]]; then
    echo -e "${RED}[FAIL] verify-purchase returned HTTP $VERIFY_HTTP${NC}"
    echo "$VERIFY_RESPONSE" | head -n -1
    exit 1
fi
echo -e "${GREEN}[OK] verify-purchase returned HTTP 200${NC}"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND external_user_id = '$USER_ID';" "1" "Subscription created by verify-purchase"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" "1" "Initial payment row created"
VERIFY_EVENT_ID=$(bridge_sql "SELECT provider_webhook_id FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'verify_purchase.succeeded' ORDER BY created_at DESC LIMIT 1;" | tr -d '[:space:]')
echo -e "${BLUE}Verify callback event id: ${VERIFY_EVENT_ID:-<none>}${NC}"
echo ""

echo -e "${YELLOW}[4/7] Sending three renewal webhooks with the same purchase token${NC}"
send_google_subscription_webhook 2 "$RENEWAL_ID_1" "$((BASE_EVENT_MS + 60000))"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'SUBSCRIPTION_RENEWED' AND processed = true;" "1" "Renewal 1 processed"

send_google_subscription_webhook 2 "$RENEWAL_ID_2" "$((BASE_EVENT_MS + 120000))"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'SUBSCRIPTION_RENEWED' AND processed = true;" "2" "Renewal 2 processed"

send_google_subscription_webhook 2 "$RENEWAL_ID_3" "$((BASE_EVENT_MS + 180000))"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'SUBSCRIPTION_RENEWED' AND processed = true;" "3" "Renewal 3 processed"
echo ""

echo -e "${YELLOW}[5/7] Sending cancellation and expiration webhooks${NC}"
send_google_subscription_webhook 3 "$CANCELED_ID" "$((BASE_EVENT_MS + 240000))"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_webhook_id = '$CANCELED_ID' AND processed = true;" "1" "Cancellation processed"

send_google_subscription_webhook 13 "$EXPIRED_ID" "$((BASE_EVENT_MS + 300000))"
wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_webhook_id = '$EXPIRED_ID' AND processed = true;" "1" "Expiration processed"
echo ""

echo -e "${YELLOW}[6/7] Validating Bridge database state${NC}"
SUB_STATUS=$(bridge_sql "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN';" | tr -d '[:space:]')
SUB_PERIOD_END=$(bridge_sql "SELECT current_period_end IS NOT NULL FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN';" | tr -d '[:space:]')
PAYMENT_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" | tr -d '[:space:]')
DISTINCT_TX_COUNT=$(bridge_sql "SELECT COUNT(DISTINCT provider_transaction_id) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" | tr -d '[:space:]')
TOKEN_TX_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND provider_transaction_id = '$PURCHASE_TOKEN';" | tr -d '[:space:]')
NULL_PRODUCT_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND product_id IS NULL;" | tr -d '[:space:]')
RENEWAL_TOKEN_TX_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND provider_transaction_id = '$PURCHASE_TOKEN' AND webhook_received_at > (SELECT MIN(webhook_received_at) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID');" | tr -d '[:space:]')

echo "Subscription status: $SUB_STATUS"
echo "Subscription current_period_end present: $SUB_PERIOD_END"
echo "Payment rows: $PAYMENT_COUNT"
echo "Distinct transaction ids: $DISTINCT_TX_COUNT"
echo "Payment rows using purchase token as tx id: $TOKEN_TX_COUNT"
echo "Payment rows with NULL product_id: $NULL_PRODUCT_COUNT"
echo "Renewal rows using purchase token as tx id: $RENEWAL_TOKEN_TX_COUNT"
echo ""

BRIDGE_CORE_OK="false"
if [[ "$SUB_STATUS" == "expired" && "$SUB_PERIOD_END" == "t" && "$PAYMENT_COUNT" == "4" && "$DISTINCT_TX_COUNT" == "4" && "$RENEWAL_TOKEN_TX_COUNT" == "0" ]]; then
    BRIDGE_CORE_OK="true"
    echo -e "${GREEN}[OK] Bridge core lifecycle state is valid${NC}"
else
    echo -e "${RED}[FAIL] Bridge core lifecycle state is invalid${NC}"
fi

INITIAL_PAYMENT_OK="false"
if [[ "$TOKEN_TX_COUNT" == "0" && "$NULL_PRODUCT_COUNT" == "0" ]]; then
    INITIAL_PAYMENT_OK="true"
    echo -e "${GREEN}[OK] Initial payment uses provider order id and product_id${NC}"
else
    echo -e "${RED}[FAIL] Initial payment still uses purchase token and/or NULL product_id${NC}"
fi
echo ""

echo -e "${YELLOW}[7/7] Optional HiHa callback database validation${NC}"
HIHA_AVAILABLE="false"
HIHA_CALLBACK_COUNT="0"
HIHA_NULL_PERIOD_END_COUNT="0"
HIHA_USER_PREMIUM="unknown"
HIHA_USER_EXPIRES_AT="unknown"
HIHA_MAX_CALLBACK_PERIOD_END="unknown"
HIHA_OK="skipped"

detect_hiha_db_mode >/dev/null 2>&1 || true
if [[ -n "$HIHA_DB_MODE" ]] && HIHA_CALLBACK_COUNT=$(hiha_sql "SELECT COUNT(*) FROM hiha.webhook_callbacks WHERE clerk_id = '$USER_ID' AND (event_id LIKE 'google_play-%' OR event_id LIKE 'verify-purchase-%');" 2>/dev/null | tr -d '[:space:]'); then
    HIHA_AVAILABLE="true"
    HIHA_NULL_PERIOD_END_COUNT=$(hiha_sql "SELECT COUNT(*) FROM hiha.webhook_callbacks WHERE clerk_id = '$USER_ID' AND (event_id LIKE 'google_play-%' OR event_id LIKE 'verify-purchase-%') AND current_period_end IS NULL;" | tr -d '[:space:]')
    HIHA_USER_PREMIUM=$(hiha_sql "SELECT is_premium FROM hiha.users WHERE clerk_id = '$USER_ID';" | tr -d '[:space:]')
    HIHA_USER_EXPIRES_AT=$(hiha_sql "SELECT COALESCE(premium_expires_at::text, '') FROM hiha.users WHERE clerk_id = '$USER_ID';" | tr -d '[:space:]')
    HIHA_MAX_CALLBACK_PERIOD_END=$(hiha_sql "SELECT COALESCE(MAX(current_period_end)::text, '') FROM hiha.webhook_callbacks WHERE clerk_id = '$USER_ID' AND (event_id LIKE 'google_play-%' OR event_id LIKE 'verify-purchase-%');" | tr -d '[:space:]')

    echo "HiHa callback rows: $HIHA_CALLBACK_COUNT"
    echo "HiHa callbacks with NULL current_period_end: $HIHA_NULL_PERIOD_END_COUNT"
    echo "HiHa user is_premium: $HIHA_USER_PREMIUM"
    echo "HiHa user premium_expires_at: $HIHA_USER_EXPIRES_AT"
    echo "HiHa max callback current_period_end: $HIHA_MAX_CALLBACK_PERIOD_END"

    if [[ "$HIHA_CALLBACK_COUNT" == "6" && "$HIHA_NULL_PERIOD_END_COUNT" == "0" && "$HIHA_USER_PREMIUM" == "f" && "$HIHA_USER_EXPIRES_AT" == "$HIHA_MAX_CALLBACK_PERIOD_END" ]]; then
        HIHA_OK="true"
        echo -e "${GREEN}[OK] HiHa callback state is valid${NC}"
    else
        HIHA_OK="false"
        echo -e "${RED}[FAIL] HiHa callback state is invalid${NC}"
    fi
else
    echo -e "${YELLOW}[WARN] HiHa DB unavailable or callback rows absent; skipped app-side assertions${NC}"
fi
echo ""

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
if [[ "$BRIDGE_CORE_OK" == "true" && "$INITIAL_PAYMENT_OK" == "true" && "$HIHA_OK" != "false" ]]; then
    TEST_STATUS="pass"
else
    TEST_STATUS="fail"
fi

cat > "$REPORT_FILE" <<EOF
{
  "test_id": "REN-01",
  "test_name": "Google Play Full Renewal Lifecycle Regression",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "bridge": {
    "core_lifecycle_ok": $BRIDGE_CORE_OK,
    "initial_payment_ok": $INITIAL_PAYMENT_OK,
    "subscription_status": "$SUB_STATUS",
    "subscription_current_period_end_present": "$SUB_PERIOD_END",
    "payment_count": $PAYMENT_COUNT,
    "distinct_transaction_count": $DISTINCT_TX_COUNT,
    "purchase_token_transaction_count": $TOKEN_TX_COUNT,
    "null_product_id_payment_count": $NULL_PRODUCT_COUNT,
    "renewal_purchase_token_transaction_count": $RENEWAL_TOKEN_TX_COUNT
  },
  "hiha": {
    "available": $HIHA_AVAILABLE,
    "ok": "$HIHA_OK",
    "callback_count": "$HIHA_CALLBACK_COUNT",
    "null_period_end_count": "$HIHA_NULL_PERIOD_END_COUNT",
    "user_is_premium": "$HIHA_USER_PREMIUM",
    "user_premium_expires_at": "$HIHA_USER_EXPIRES_AT",
    "max_callback_current_period_end": "$HIHA_MAX_CALLBACK_PERIOD_END"
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}[OK] REN-01 Test PASSED${NC}"
else
    echo -e "${RED}[FAIL] REN-01 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
