#!/bin/bash

##############################################################################
# WHK-06: Google Play Renewal Message Deduplication
# 
# Purpose: Verify that two Google Play renewal webhooks with different
#          message_id values but the same purchase_token + event type are
#          both processed as distinct renewals.
#
# Usage: ./test-whk-06.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN/test-token
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# Expected Behavior:
#   - Same message_id is idempotent.
#   - Different message_id is a distinct Google Play renewal, even when
#     purchase_token and event_type are reused.
#   - pay.webhook_provider stores both renewal message ids.
#   - pay.payments uses a per-event/order transaction id, not the purchase token.
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="whk-06-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
APP_ID="$BRIDGE_APP_ID"
REPORT_FILE="whk-06-report.json"
USER_ID="${USER_ID:-test_whk_06_user_$TEST_RUN_ID}"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "WHK-06: Google Play Renewal Message Deduplication"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

echo -e "${YELLOW}[1/3] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}[FAIL] Failed to prepare user_id${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}[OK] User ID: $USER_ID${NC}"
echo ""

echo -e "${YELLOW}[2/3] Setting up test subscription${NC}"

PURCHASE_TOKEN="test-whk-06-token-$TEST_RUN_ID"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token LIKE 'test-whk-06%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND (provider_transaction_id LIKE 'mock-google-play-renewal:test-whk-06%' OR external_user_id = '$USER_ID');" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" 2>/dev/null

INSERT_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true, NOW(), NOW()) ON CONFLICT (app_id, external_user_id, subscription_id, provider) DO UPDATE SET status = 'active', purchase_token = EXCLUDED.purchase_token, auto_renewing = EXCLUDED.auto_renewing, updated_at = NOW();" 2>&1)
if [[ "$INSERT_SUB" == *"ERROR"* ]] || [[ "$INSERT_SUB" == *"error"* ]]; then
    echo -e "${RED}[FAIL] Failed to insert subscription fixture${NC}"
    echo "$INSERT_SUB"
    exit 1
fi

echo -e "${GREEN}[OK] Created subscription fixture${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

echo -e "${YELLOW}[3/3] Sending distinct renewal webhooks${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

MESSAGE_ID_1="msg-whk-06-1-$TEST_RUN_ID"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_1\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_1=$(echo "$RESPONSE_1" | tail -n1)
echo "First webhook response: HTTP $HTTP_CODE_1"
if [[ "$HTTP_CODE_1" != "200" && "$HTTP_CODE_1" != "204" ]]; then
    echo "Response body: $(echo "$RESPONSE_1" | head -n -1)"
    echo -e "${RED}[FAIL] First webhook failed${NC}"
    exit 1
fi

sleep 2

COUNT_1=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')

MESSAGE_ID_2="msg-whk-06-2-$TEST_RUN_ID"
# Regenerate notification JSON with new timestamp to bypass stale event suppression
NOTIFICATION_JSON_2=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64_2=$(echo -n "$NOTIFICATION_JSON_2" | base64 -w 0)

RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64_2\", \"message_id\": \"$MESSAGE_ID_2\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_2=$(echo "$RESPONSE_2" | tail -n1)
echo "Second webhook response: HTTP $HTTP_CODE_2"
if [[ "$HTTP_CODE_2" != "200" && "$HTTP_CODE_2" != "204" ]]; then
    echo "Response body: $(echo "$RESPONSE_2" | head -n -1)"
    echo -e "${RED}[FAIL] Second webhook failed${NC}"
    exit 1
fi

sleep 2

COUNT_2=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
WEBHOOK_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'SUBSCRIPTION_RENEWED';" -t 2>/dev/null | tr -d ' ')
TOKEN_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "After first webhook: $COUNT_1 payment record(s)"
echo "After second webhook: $COUNT_2 payment record(s)"
echo "Webhook dedup rows for token/type: $WEBHOOK_COUNT"
echo "Payments using purchase token as transaction id: $TOKEN_PAYMENT_COUNT"
echo ""

if [[ "$COUNT_1" == "1" ]] && [[ "$COUNT_2" == "2" ]] && [[ "$WEBHOOK_COUNT" == "2" ]] && [[ "$TOKEN_PAYMENT_COUNT" == "0" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}[OK] WHK-06 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}[FAIL] WHK-06 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "WHK-06",
  "test_name": "Google Play Renewal Message Deduplication",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "first_webhook_message_id": "$MESSAGE_ID_1",
    "first_webhook_http_code": $HTTP_CODE_1,
    "payment_count_after_first": $COUNT_1,
    "second_webhook_message_id": "$MESSAGE_ID_2",
    "second_webhook_http_code": $HTTP_CODE_2,
    "payment_count_after_second": $COUNT_2,
    "webhook_rows_for_token_and_type": $WEBHOOK_COUNT,
    "purchase_token_payment_count": $TOKEN_PAYMENT_COUNT,
    "distinct_renewals_processed": $([[ "$COUNT_1" == "1" ]] && [[ "$COUNT_2" == "2" ]] && [[ "$WEBHOOK_COUNT" == "2" ]] && [[ "$TOKEN_PAYMENT_COUNT" == "0" ]] && echo "true" || echo "false")
  },
  "notes": "Distinct Google Play renewal message ids should not collapse by purchase_token + event_type."
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
