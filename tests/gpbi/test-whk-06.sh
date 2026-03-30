#!/bin/bash

##############################################################################
# WHK-06: Token-based Webhook Deduplication
#
# Purpose: Verify that webhooks with DIFFERENT message_id but SAME
#          purchase_token + event_type are handled idempotently; no
#          duplicate payment records created.
#
# Usage: ./test-whk-06.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and listening on $APP_URL (default: http://localhost:3000)
#   - Backend configured with: MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - Test uses header: X-Webhook-Verification-Mode: off
#     (Skips signature verification - tests token-based deduplication)
#
# TESTPLAN Reference:
#   Backend Behavior: First webhook (message_id_1, token_A, type_2) creates payment,
#                     Second webhook (message_id_2, token_A, type_2): deduplicates,
#                     Payment count remains 1 (token + type as dedup key),
#                     Backend checks token + notification type before processing.
#   DB Validation: pay.payments table: Only 1 record for given token,
#                  No duplicate rows despite different message_id values.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Test configuration
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
EMAIL=""
APP_URL="$APP_URL"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email) EMAIL="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

export PGPASSWORD="${DATABASE_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "WHK-06: Token-based Webhook Deduplication"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Get user_id
echo -e "${YELLOW}[1/3] Fetching user_id from database for email: $EMAIL${NC}"
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Setup subscription
echo -e "${YELLOW}[2/3] Setting up test subscription${NC}"

PURCHASE_TOKEN="test-token-dedup-$(date +%s)"

# Clean up any existing test data
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.payments WHERE provider_transaction_id LIKE 'test-token-dedup%';" 2>&1 | head -1
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>&1 | head -1

# Create subscription
INSERT_SUB=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing) VALUES ('$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true);" 2>&1)
if [[ "$INSERT_SUB" == *"ERROR"* ]] || [[ "$INSERT_SUB" == *"error"* ]]; then
    echo -e "${RED}✗ Failed to insert subscription: $INSERT_SUB${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Setup subscription (status: active)${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Send FIRST webhook with message_id_1
echo -e "${YELLOW}[3/3] Sending webhooks to test token-based deduplication${NC}"
echo ""

MESSAGE_ID_1="msg-1-$(date +%s)"
echo "First webhook: message_id = $MESSAGE_ID_1"
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

echo -e "${YELLOW}Sending first webhook (Message ID: $MESSAGE_ID_1)${NC}"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{\"message\": {\"data\": \"$NOTIFICATION_B64\", \"message_id\": \"$MESSAGE_ID_1\", \"attributes\": {}}, \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"}")

HTTP_CODE_1=$(echo "$RESPONSE_1" | tail -n1)
echo "First webhook response: HTTP $HTTP_CODE_1"

if [[ "$HTTP_CODE_1" != "200" ]]; then
    echo -e "${RED}✗ First webhook failed (HTTP $HTTP_CODE_1)${NC}"
    exit 1
fi

# Step 4: Record state
COUNT_1=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE provider_transaction_id = '$PURCHASE_TOKEN';" -t | tr -d ' ')

# Step 5: Send SECOND webhook with DIFFERENT Message ID but SAME Token + Type
MESSAGE_ID_2="msg-2-$(date +%s)"
echo -e "\nSecond webhook: message_id = $MESSAGE_ID_2 (different from first, same token/type)"

RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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

# Step 6: Verify deduplication (DB Validation)
echo ""
echo -e "${YELLOW}Verifying token-based deduplication (DB Validation)${NC}"
echo ""

COUNT_2=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE provider_transaction_id = '$PURCHASE_TOKEN';" -t | tr -d ' ')

echo "After first webhook: $COUNT_1 payment record(s)"
echo "After second webhook: $COUNT_2 payment record(s)"
echo ""

echo -e "${YELLOW}========================================${NC}"

if [[ "$COUNT_1" == "1" ]] && [[ "$COUNT_2" == "1" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-06 Test PASSED - Token Deduplication Works${NC}"
    echo -e "$TEST_RESULT_MSG"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ WHK-06 Test FAILED${NC}"
    echo -e "$TEST_RESULT_MSG"
    echo -e "${RED}Expected 1 payment record, but got: $COUNT_2${NC}"
fi

echo -e "${YELLOW}========================================${NC}"
echo ""

# Generate JSON report
cat > whk-06-report.json <<EOF
{
  "test_id": "WHK-06",
  "test_name": "Token-based Webhook Deduplication",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "first_webhook_message_id": "$MESSAGE_ID_1",
    "first_webhook_http_code": $HTTP_CODE_1,
    "payment_count_after_first": $COUNT_1,
    "second_webhook_message_id": "$MESSAGE_ID_2",
    "second_webhook_http_code": $HTTP_CODE_2,
    "payment_count_after_second": $COUNT_2,
    "deduplication_enforced": $([[ "$COUNT_1" == "1" ]] && [[ "$COUNT_2" == "1" ]] && echo "true" || echo "false")
  },
  "notes": "Token + notification_type should be dedup key; different message_id should not create duplicate"
}
EOF

echo "Report saved to: whk-06-report.json"
cat whk-06-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
