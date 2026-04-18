#!/bin/bash

##############################################################################
# WHK-06: Token-based Webhook Deduplication
# 
# Purpose: Verify that two Google Play renewal webhooks with different
#          message_id values but the same logical purchase_token + event
#          combination are deduplicated and do not create duplicate rows.
#
# Usage: ./test-whk-06.sh
#
# Prerequisites:
#   - Backend running and listening on $BRIDGE_API_URL
#   - Backend configured with: MOCK_EXTERNAL_APIS=true
#   - Bridge database accessible (credentials via globals.cfg)
#   - psql installed and in PATH
#   - Test uses header: X-Webhook-Verification-Mode: off
#
# TESTPLAN Reference:
#   Backend Behavior: Backend checks (purchase_token, event_type) combination.
#                     If already processed successfully, skips logic even if message_id is new.
#                     Prevents double-processing if Google sends same logical event.
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
APP_ID="$BRIDGE_APP_ID"
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_whk_06_user_$RUN_ID}"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "WHK-06: Token-based Webhook Deduplication"
echo -e "${YELLOW}========================================${NC}"
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

PURCHASE_TOKEN="test-token-dedup-$(date +%s)"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token LIKE 'test-token-dedup%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id LIKE 'test-token-dedup%';" 2>/dev/null
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

echo -e "${YELLOW}[3/3] Sending duplicate renewal webhooks${NC}"

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

MESSAGE_ID_1="msg-1-$(date +%s)"
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

COUNT_1=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

MESSAGE_ID_2="msg-2-$(date +%s)"
RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_2\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_2=$(echo "$RESPONSE_2" | tail -n1)
echo "Second webhook response: HTTP $HTTP_CODE_2"
if [[ "$HTTP_CODE_2" != "200" && "$HTTP_CODE_2" != "204" ]]; then
    echo "Response body: $(echo "$RESPONSE_2" | head -n -1)"
    echo -e "${RED}[FAIL] Second webhook failed${NC}"
    exit 1
fi

sleep 2

COUNT_2=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
WEBHOOK_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'SUBSCRIPTION_RENEWED';" -t 2>/dev/null | tr -d ' ')

echo "After first webhook: $COUNT_1 payment record(s)"
echo "After second webhook: $COUNT_2 payment record(s)"
echo "Webhook dedup rows for token/type: $WEBHOOK_COUNT"
echo ""

if [[ "$COUNT_1" == "1" ]] && [[ "$COUNT_2" == "1" ]] && [[ "$WEBHOOK_COUNT" == "1" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}[OK] WHK-06 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}[FAIL] WHK-06 Test FAILED${NC}"
fi

cat > whk-06-report.json <<EOF
{
  "test_id": "WHK-06",
  "test_name": "Token-based Webhook Deduplication",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
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
    "deduplication_enforced": $([[ "$COUNT_1" == "1" ]] && [[ "$COUNT_2" == "1" ]] && [[ "$WEBHOOK_COUNT" == "1" ]] && echo "true" || echo "false")
  },
  "notes": "The duplicate renewal should collapse to one token/event ingress row and one payment row."
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-06-report.json"
cat whk-06-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
