#!/bin/bash

##############################################################################
# WHK-05: Refund Idempotency Verification
# 
# Purpose: Verify that second refund webhooks for the SAME token but a 
#          DIFFERENT message_id skip re-revocation logic.
#
# Usage: ./test-whk-05.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Second refund webhook results in HTTP 200/204.
#                      No duplicate records or re-applied state.
#                      Backend logic detects previous 'VOIDED_PURCHASE' processing.
#                      Ensures robust idempotency across different deliveries.
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
USER_ID="${USER_ID:-test_whk_05_user_$RUN_ID}"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "WHK-05: Refund Idempotency Verification"
echo -e "${YELLOW}========================================${NC}"
echo ""

echo -e "${YELLOW}[1/4] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}[FAIL] Failed to prepare user_id${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}[OK] User ID: $USER_ID${NC}"
echo ""

echo -e "${YELLOW}[2/4] Setting up payment and subscription fixtures${NC}"

PURCHASE_TOKEN="test-refund-idem-$(date +%s)"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token LIKE 'test-refund-idem%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id LIKE 'test-refund-idem%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" 2>/dev/null

INSERT_PAYMENT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.payments (app_id, external_user_id, provider, provider_transaction_id, subscription_id, status, amount_cents) VALUES ('$APP_ID', '$USER_ID', '$PROVIDER', '$PURCHASE_TOKEN', '$PRODUCT_ID', 'success', 1000) ON CONFLICT (app_id, provider, provider_transaction_id) DO UPDATE SET external_user_id = EXCLUDED.external_user_id, subscription_id = EXCLUDED.subscription_id, status = EXCLUDED.status, amount_cents = EXCLUDED.amount_cents;" 2>&1)
if [[ "$INSERT_PAYMENT" == *"ERROR"* ]] || [[ "$INSERT_PAYMENT" == *"error"* ]]; then
    echo -e "${RED}[FAIL] Failed to insert payment fixture${NC}"
    echo "$INSERT_PAYMENT"
    exit 1
fi

INSERT_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true, NOW(), NOW()) ON CONFLICT (app_id, external_user_id, subscription_id, provider) DO UPDATE SET status = 'active', purchase_token = EXCLUDED.purchase_token, auto_renewing = EXCLUDED.auto_renewing, updated_at = NOW();" 2>&1)
if [[ "$INSERT_SUB" == *"ERROR"* ]] || [[ "$INSERT_SUB" == *"error"* ]]; then
    echo -e "${RED}[FAIL] Failed to insert subscription fixture${NC}"
    echo "$INSERT_SUB"
    exit 1
fi

echo -e "${GREEN}[OK] Created payment + subscription fixtures${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "voidedPurchaseNotification": {
    "purchaseToken": "$PURCHASE_TOKEN",
    "orderId": "GPA.1111-2222-3333-44444",
    "productType": 1,
    "refundType": 0
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

echo -e "${YELLOW}[3/4] Sending first refund webhook${NC}"

MESSAGE_ID_1="msg-refund-1-$(date +%s)"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_1\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_1=$(echo "$RESPONSE_1" | tail -n1)
echo "First webhook response: HTTP $HTTP_CODE_1"
if [[ "$HTTP_CODE_1" != "200" && "$HTTP_CODE_1" != "204" ]]; then
    echo "Response body: $(echo "$RESPONSE_1" | head -n -1)"
    echo -e "${RED}[FAIL] First refund webhook failed${NC}"
    exit 1
fi

sleep 2

PAYMENT_STATUS_1=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
SUB_STATUS_1=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

echo "After first refund: payment status = $PAYMENT_STATUS_1, subscription status = $SUB_STATUS_1"
echo ""

echo -e "${YELLOW}[4/4] Sending second refund webhook with a different message_id${NC}"

MESSAGE_ID_2="msg-refund-2-$(date +%s)"
RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_2\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_2=$(echo "$RESPONSE_2" | tail -n1)
echo "Second webhook response: HTTP $HTTP_CODE_2"
if [[ "$HTTP_CODE_2" != "200" && "$HTTP_CODE_2" != "204" ]]; then
    echo "Response body: $(echo "$RESPONSE_2" | head -n -1)"
    echo -e "${RED}[FAIL] Second refund webhook failed${NC}"
    exit 1
fi

sleep 2

PAYMENT_STATUS_2=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND provider_transaction_id = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
SUB_STATUS_2=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')
WEBHOOK_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$PURCHASE_TOKEN' AND event_type = 'VOIDED_PURCHASE';" -t 2>/dev/null | tr -d ' ')

echo "After second refund: payment status = $PAYMENT_STATUS_2, subscription status = $SUB_STATUS_2"
echo "Webhook dedup rows for token/type: $WEBHOOK_COUNT"
echo ""

if [[ "$PAYMENT_STATUS_1" == "refunded" ]] && [[ "$PAYMENT_STATUS_2" == "refunded" ]] && [[ "$SUB_STATUS_1" == "revoked" ]] && [[ "$SUB_STATUS_2" == "revoked" ]] && [[ "$WEBHOOK_COUNT" == "1" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}[OK] WHK-05 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}[FAIL] WHK-05 Test FAILED${NC}"
fi

cat > whk-05-report.json <<EOF
{
  "test_id": "WHK-05",
  "test_name": "Refund Idempotency Verification",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "first_refund_webhook_http_code": $HTTP_CODE_1,
    "first_refund_payment_status": "$PAYMENT_STATUS_1",
    "first_refund_subscription_status": "$SUB_STATUS_1",
    "second_refund_webhook_http_code": $HTTP_CODE_2,
    "second_refund_payment_status": "$PAYMENT_STATUS_2",
    "second_refund_subscription_status": "$SUB_STATUS_2",
    "webhook_rows_for_token_and_type": $WEBHOOK_COUNT,
    "idempotency_enforced": $([[ "$PAYMENT_STATUS_1" == "refunded" ]] && [[ "$PAYMENT_STATUS_2" == "refunded" ]] && [[ "$SUB_STATUS_1" == "revoked" ]] && [[ "$SUB_STATUS_2" == "revoked" ]] && [[ "$WEBHOOK_COUNT" == "1" ]] && echo "true" || echo "false")
  },
  "notes": "Repeated refund webhooks should not create duplicate token/event records or re-apply refund state."
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-05-report.json"
cat whk-05-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
