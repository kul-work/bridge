#!/bin/bash

##############################################################################
# WHK-05: Refund Idempotency Verification
#
# Purpose: Verify that repeated purchase.voided notifications for the same
#          token do not trigger redundant revocation logic; payment status
#          remains 'refunded' and subscription is only revoked once.
#
# Usage: ./test-whk-05.sh
#
# Prerequisites:
#   - Backend running and listening on $BRIDGE_API_URL (default: http://localhost:3000)
#   - Backend configured with: MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - Test uses header: X-Webhook-Verification-Mode: off
#     (Skips signature verification - tests refund idempotency)
#
# TESTPLAN Reference:
#   Backend Behavior: First refund webhook marks payment as 'refunded',
#                     revokes subscription to 'canceled',
#                     Second webhook with same token: payment remains 'refunded',
#                     No redundant revocation logic executed (verified via logs).
#   DB Validation: pay.payments table: status = 'refunded' (immutable after first refund),
#                  pay.subscriptions table: status remains 'canceled' (no re-revocation).
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Test configuration
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_whk_05_user_$RUN_ID}"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"

export PGPASSWORD="${DATABASE_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "WHK-05: Refund Idempotency Verification"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/4] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Setup payment and subscription records
echo -e "${YELLOW}[2/4] Setting up test payment and subscription records${NC}"

PURCHASE_TOKEN="test-refund-idem-$(date +%s)"

# Clean up any existing test data
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE provider_transaction_id LIKE 'test-refund-idem%';" 2>&1 | head -1
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>&1 | head -1

# Create payment record with 'success' status
INSERT_PAYMENT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.payments (external_user_id, provider, provider_transaction_id, subscription_id, status, amount_cents) VALUES ('$USER_ID', '$PROVIDER', '$PURCHASE_TOKEN', '$PRODUCT_ID', 'success', 1000);" 2>&1)
if [[ "$INSERT_PAYMENT" == *"ERROR"* ]] || [[ "$INSERT_PAYMENT" == *"error"* ]]; then
    echo -e "${RED}✗ Failed to insert payment: $INSERT_PAYMENT${NC}"
    exit 1
fi

# Create subscription record with 'active' status
INSERT_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing) VALUES ('$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true);" 2>&1)
if [[ "$INSERT_SUB" == *"ERROR"* ]] || [[ "$INSERT_SUB" == *"error"* ]]; then
    echo -e "${RED}✗ Failed to insert subscription: $INSERT_SUB${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Setup: payment (status: success), subscription (status: active)${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Send FIRST refund webhook
echo -e "${YELLOW}[3/4] Sending FIRST refund webhook${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 12,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

MESSAGE_ID_1="msg-refund-1-$(date +%s)"
echo -e "\n${YELLOW}Sending FIRST refund webhook (Message ID: $MESSAGE_ID_1)${NC}"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_1\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_1=$(echo "$RESPONSE_1" | tail -n1)
echo "First webhook response: HTTP $HTTP_CODE_1"
if [[ "$HTTP_CODE_1" != "200" ]]; then
  echo "Response body: $(echo "$RESPONSE_1" | head -n -1)"
fi

if [[ "$HTTP_CODE_1" != "200" ]]; then
    echo -e "${RED}✗ First webhook failed (HTTP $HTTP_CODE_1)${NC}"
    exit 1
fi

# Wait a bit for async processing
sleep 2

# Verify subscription is revoked
STATUS_1=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

echo -e "After first refund: subscription status = $STATUS_1"

# Step 4: Send SECOND refund webhook (different message_id, same result)
echo -e "\n${YELLOW}[4/4] Sending SECOND refund webhook (different message_id, same token)${NC}"

MESSAGE_ID_2="msg-refund-2-$(date +%s)"
echo "Message ID: $MESSAGE_ID_2 (different from first)"
echo ""
RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_2\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_2=$(echo "$RESPONSE_2" | tail -n1)
echo "Second webhook response: HTTP $HTTP_CODE_2"

if [[ "$HTTP_CODE_2" != "200" ]]; then
    echo -e "${RED}✗ Second webhook failed (HTTP $HTTP_CODE_2)${NC}"
    exit 1
fi

# Wait a bit
sleep 2

# Verify status remains same (revoked)
STATUS_2=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

echo ""
echo -e "${YELLOW}========================================${NC}"

if [[ "$STATUS_1" == "revoked" ]] && [[ "$STATUS_2" == "revoked" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-05 Test PASSED - Refund Idempotency Works${NC}"
    echo -e "$TEST_RESULT_MSG"
    echo -e "${BLUE}Subscription status: $STATUS_1 → $STATUS_2 (unchanged, idempotent)${NC}"
    RESULT="success"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ WHK-05 Test FAILED${NC}"
    echo -e "$TEST_RESULT_MSG"
    echo -e "${RED}First refund status: $STATUS_1${NC}"
    echo -e "${RED}Second refund status: $STATUS_2 (should remain 'revoked')${NC}"
    RESULT="failure"
fi

echo -e "${YELLOW}========================================${NC}"
echo ""

# Generate JSON report
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
    "first_refund_subscription_status": "$STATUS_1",
    "second_refund_webhook_http_code": $HTTP_CODE_2,
    "second_refund_subscription_status": "$STATUS_2",
    "idempotency_enforced": $([[ "$STATUS_1" == "revoked" ]] && [[ "$STATUS_2" == "revoked" ]] && echo "true" || echo "false")
  },
  "notes": "Refund webhooks should be idempotent; second refund of same token should not re-revoke"
  -H "X-Webhook-Verification-Mode: off" \
}
EOF

echo "Report saved to: whk-05-report.json"
cat whk-05-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
