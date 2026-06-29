#!/bin/bash

##############################################################################
# WHK-07: Google Play Same-SKU Cross-User Webhook Isolation
#
# Purpose: Verify that a Google Play webhook for one purchase token is not
#          evaluated against another user's newer row that shares the same
#          subscription_id/SKU.
#
# Usage: ./test-whk-07.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN or WEBHOOK_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# Expected Behavior:
#   - Two rows may share the Google subscription_id because it is a product SKU.
#   - The webhook for token A is not suppressed because token B has a newer
#     last_event_time on the same SKU.
#   - Token B's row remains untouched.
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="whk-07-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
APP_ID="$BRIDGE_APP_ID"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
WEBHOOK_PATH_TOKEN="${WEBHOOK_INGRESS_TOKEN:-${WEBHOOK_TOKEN:-}}"
REPORT_FILE="whk-07-report.json"
USER_A="test_whk_07_user_a_$TEST_RUN_ID"
USER_B="test_whk_07_user_b_$TEST_RUN_ID"
TOKEN_A="test-whk-07-token-a-$TEST_RUN_ID"
TOKEN_B="test-whk-07-token-b-$TEST_RUN_ID"
MESSAGE_ID="msg-whk-07-$TEST_RUN_ID"
EVENT_TIME_MS=1500000
ROW_A_LAST_EVENT=1000000
ROW_B_LAST_EVENT=2000000

DB_URL="${BRIDGE_DB_URL}"
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "WHK-07: Google Play Same-SKU Cross-User Webhook Isolation"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if [[ -z "$APP_ID" || -z "$WEBHOOK_PATH_TOKEN" ]]; then
    echo -e "${RED}[FAIL] Missing BRIDGE_APP_ID or webhook ingress token${NC}"
    exit 1
fi

cleanup() {
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.webhook_delivery WHERE webhook_provider_id IN (SELECT id FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider_webhook_id LIKE 'msg-whk-07-%');" >/dev/null 2>&1 || true
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider_webhook_id LIKE 'msg-whk-07-%';" >/dev/null 2>&1 || true
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id LIKE 'test_whk_07_%';" >/dev/null 2>&1 || true
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND (external_user_id LIKE 'test_whk_07_%' OR purchase_token LIKE 'test-whk-07-%');" >/dev/null 2>&1 || true
}

cleanup

echo -e "${YELLOW}[1/4] Seeding two users on the same Google Play SKU${NC}"

INSERT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -v ON_ERROR_STOP=1 -c "
INSERT INTO pay.subscriptions
    (app_id, external_user_id, subscription_id, provider, purchase_token, status, auto_renewing, last_event_time, created_at, updated_at)
VALUES
    ('$APP_ID', '$USER_A', '$PRODUCT_ID', '$PROVIDER', '$TOKEN_A', 'active', true, $ROW_A_LAST_EVENT, NOW() - INTERVAL '2 minutes', NOW()),
    ('$APP_ID', '$USER_B', '$PRODUCT_ID', '$PROVIDER', '$TOKEN_B', 'cancelled', false, $ROW_B_LAST_EVENT, NOW() - INTERVAL '1 minute', NOW());
" 2>&1)

if [[ "$INSERT_RESULT" == *"ERROR"* || "$INSERT_RESULT" == *"error"* ]]; then
    echo -e "${RED}[FAIL] Failed to insert same-SKU subscription fixtures${NC}"
    echo "$INSERT_RESULT"
    exit 1
fi

echo -e "${GREEN}[OK] Seeded token A and token B on SKU $PRODUCT_ID${NC}"
echo ""

echo -e "${YELLOW}[2/4] Sending Google Play webhook for token A${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$EVENT_TIME_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 3,
    "purchaseToken": "$TOKEN_A",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_PATH_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
echo "Webhook response: HTTP $WEBHOOK_HTTP_CODE"
if [[ "$WEBHOOK_HTTP_CODE" != "200" && "$WEBHOOK_HTTP_CODE" != "204" ]]; then
    echo "Response body: $(echo "$WEBHOOK_RESPONSE" | head -n -1)"
    echo -e "${RED}[FAIL] Webhook request failed${NC}"
    exit 1
fi
echo ""

echo -e "${YELLOW}[3/4] Waiting for async webhook processing${NC}"

WEBHOOK_ROW=""
DELIVERY_COUNT="0"
for _ in {1..12}; do
    WEBHOOK_ROW=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT id::text || '|' || processed::text || '|' || suppressed::text || '|' || COALESCE(suppressed_reason, '') FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_webhook_id = '$MESSAGE_ID' LIMIT 1;" -t 2>/dev/null | tr -d ' ')
    DELIVERY_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_delivery WHERE webhook_provider_id IN (SELECT id FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider_webhook_id = '$MESSAGE_ID');" -t 2>/dev/null | tr -d ' ')
    if [[ "$WEBHOOK_ROW" == *"|true|"* && "$DELIVERY_COUNT" != "0" ]]; then
        break
    fi
    sleep 1
done

IFS='|' read -r WEBHOOK_ID WEBHOOK_PROCESSED WEBHOOK_SUPPRESSED WEBHOOK_SUPPRESSED_REASON <<< "$WEBHOOK_ROW"
echo "Webhook row: processed=$WEBHOOK_PROCESSED suppressed=$WEBHOOK_SUPPRESSED reason=${WEBHOOK_SUPPRESSED_REASON:-none}"
echo "Delivery rows: $DELIVERY_COUNT"
echo ""

echo -e "${YELLOW}[4/4] Verifying token B stayed isolated${NC}"

ROW_A_STATE=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT external_user_id || '|' || status || '|' || last_event_time::text FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$TOKEN_A' LIMIT 1;" -t 2>/dev/null | tr -d ' ')
ROW_B_STATE=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT external_user_id || '|' || status || '|' || last_event_time::text FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$TOKEN_B' LIMIT 1;" -t 2>/dev/null | tr -d ' ')
SAME_SKU_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND subscription_id = '$PRODUCT_ID' AND purchase_token IN ('$TOKEN_A', '$TOKEN_B');" -t 2>/dev/null | tr -d ' ')

IFS='|' read -r ROW_A_USER ROW_A_STATUS ROW_A_EVENT <<< "$ROW_A_STATE"
IFS='|' read -r ROW_B_USER ROW_B_STATUS ROW_B_EVENT <<< "$ROW_B_STATE"

echo "Token A row: user=$ROW_A_USER status=$ROW_A_STATUS last_event_time=$ROW_A_EVENT"
echo "Token B row: user=$ROW_B_USER status=$ROW_B_STATUS last_event_time=$ROW_B_EVENT"
echo "Same-SKU fixture rows: $SAME_SKU_COUNT"
echo ""

WEBHOOK_NOT_SUPPRESSED="false"
DELIVERY_CREATED="false"
ROW_B_UNTOUCHED="false"
SAME_SKU_ROWS_PRESENT="false"

if [[ "$WEBHOOK_PROCESSED" == "true" && "$WEBHOOK_SUPPRESSED" == "false" ]]; then
    WEBHOOK_NOT_SUPPRESSED="true"
fi
if [[ "$DELIVERY_COUNT" -ge 1 ]]; then
    DELIVERY_CREATED="true"
fi
if [[ "$ROW_B_USER" == "$USER_B" && "$ROW_B_STATUS" == "cancelled" && "$ROW_B_EVENT" == "$ROW_B_LAST_EVENT" ]]; then
    ROW_B_UNTOUCHED="true"
fi
if [[ "$SAME_SKU_COUNT" == "2" ]]; then
    SAME_SKU_ROWS_PRESENT="true"
fi

if [[ "$WEBHOOK_NOT_SUPPRESSED" == "true" && "$DELIVERY_CREATED" == "true" && "$ROW_B_UNTOUCHED" == "true" && "$SAME_SKU_ROWS_PRESENT" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}[OK] WHK-07 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}[FAIL] WHK-07 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "WHK-07",
  "test_name": "Google Play Same-SKU Cross-User Webhook Isolation",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "provider_webhook_id": "$MESSAGE_ID",
  "subscription_id": "$PRODUCT_ID",
  "token_a": "$TOKEN_A",
  "token_b": "$TOKEN_B",
  "results": {
    "webhook_http_code": $WEBHOOK_HTTP_CODE,
    "webhook_processed": $WEBHOOK_PROCESSED,
    "webhook_suppressed": $WEBHOOK_SUPPRESSED,
    "webhook_suppressed_reason": "${WEBHOOK_SUPPRESSED_REASON:-}",
    "delivery_count": $DELIVERY_COUNT,
    "webhook_not_suppressed": $WEBHOOK_NOT_SUPPRESSED,
    "delivery_created": $DELIVERY_CREATED,
    "same_sku_rows_present": $SAME_SKU_ROWS_PRESENT,
    "token_b_untouched": $ROW_B_UNTOUCHED,
    "token_b_status": "$ROW_B_STATUS",
    "token_b_last_event_time": $ROW_B_EVENT
  },
  "notes": "Google Play subscription_id is a shared SKU; stale suppression must use purchase_token."
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
