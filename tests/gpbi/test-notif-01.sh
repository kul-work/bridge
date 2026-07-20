#!/bin/bash

##############################################################################
# NOTIF-01: Payment Failure Notification & Acknowledgment
#
# Purpose: Verify that recoverable payment failures (e.g., account hold) 
#          trigger user notifications that can be acknowledged via API.
#
# Usage: ./test-notif-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, GCP_PROJECT_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: A SUBSCRIPTION_ACCOUNT_HOLD (5) webhook is processed.
#                      The 'payment_failure_notification' flag is set to true.
#                      A POST /api/v1/subscriptions/{id}/acknowledge call clears the flag.
#                      Ensures the notification pipeline for recoverable errors is functional.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Config
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="notif-01-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-notif-01-token-$TEST_RUN_ID"
PROVIDER="$PROVIDER"
REPORT_FILE="notif-01-report.json"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
USER_ID="${USER_ID:-test_notif_01_user_$TEST_RUN_ID}"
REGISTER_HTTP_CODE=0
VERIFY_HTTP_CODE=0
WEBHOOK_HTTP_CODE=0
STATUS_HTTP_CODE=0
ACK_HTTP_CODE=0
FINAL_STATUS_HTTP_CODE=0

fail_test() {
    local failure_step="$1"
    local details="$2"
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "NOTIF-01",
  "test_name": "Payment Failure & Acknowledgment",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "user_id": "$USER_ID",
  "failure_step": "$failure_step",
  "details": "$details",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "webhook_http_code": $WEBHOOK_HTTP_CODE,
  "status_http_code": $STATUS_HTTP_CODE,
  "ack_http_code": $ACK_HTTP_CODE,
  "final_status_http_code": $FINAL_STATUS_HTTP_CODE
}
EOF
    echo -e "${RED}NOTIF-01 failed at $failure_step: $details${NC}"
    echo "Report saved to: $REPORT_FILE"
    exit 1
}

# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "NOTIF-01: Payment Failure & Acknowledgment"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if ! command -v jq >/dev/null 2>&1; then
    fail_test "prerequisite" "jq is required for NOTIF-01 response assertions"
fi

# 1. Prepare generated user_id for this run
echo -e "${YELLOW}[1/5] Preparing generated user_id for this run${NC}"
if [[ -z "$USER_ID" ]]; then fail_test "user_id" "generated user ID is empty"; fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# 2. Initial Active Purchase
echo -e "${YELLOW}[1/5] Initial Purchase (Active)${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

# Register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-notif-01-setup\"
  }")

if [[ "$REGISTER_HTTP_CODE" != "200" ]]; then
    fail_test "register" "expected HTTP 200, got $REGISTER_HTTP_CODE"
fi

VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

if [[ "$VERIFY_HTTP_CODE" != "200" ]]; then
    fail_test "verify" "expected HTTP 200, got $VERIFY_HTTP_CODE"
fi

# 3. Simulate Account Hold (Payment Failure)
echo -e "${YELLOW}[2/5] Triggering Account Hold (Payment Failure)${NC}"
WEBHOOK_ID="webhook-notif-01-$TEST_RUN_ID"
TIMESTAMP_MS=$(date +%s%3N)
NOTIFICATION_JSON="{\"version\":\"1.0\",\"packageName\":\"$PACKAGE_NAME\",\"eventTimeMillis\":\"$TIMESTAMP_MS\",\"subscriptionNotification\":{\"version\":\"1.0\",\"notificationType\":5,\"purchaseToken\":\"$DUMMY_TOKEN\",\"subscriptionId\":\"$PRODUCT_ID\"}}"
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{\"message\":{\"data\":\"$NOTIFICATION_B64\",\"message_id\":\"$WEBHOOK_ID\"},\"subscription\":\"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"}")

if [[ "$WEBHOOK_HTTP_CODE" != "200" && "$WEBHOOK_HTTP_CODE" != "204" ]]; then
    fail_test "webhook" "expected HTTP 200 or 204, got $WEBHOOK_HTTP_CODE"
fi

sleep 2

# 4. Verify Notification Flag (TRUE)
echo -e "${YELLOW}[3/5] Verifying Notification Flag = TRUE${NC}"
STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0")
STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
STATUS_RESP=$(echo "$STATUS_RESPONSE" | sed '$d')

if [[ "$STATUS_HTTP_CODE" != "200" ]]; then
    fail_test "notification_status" "expected HTTP 200, got $STATUS_HTTP_CODE"
fi

FLAG=$(echo "$STATUS_RESP" | jq -r '.payment_failure_notification')

if [[ "$FLAG" == "true" ]]; then
    echo -e "${GREEN}✓ Notification active${NC}"
else
    fail_test "notification_flag_true" "expected true, got $FLAG"
fi

# 5. Acknowledge
echo -e "${YELLOW}[4/5] Acknowledging Notification${NC}"
ACK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/subscriptions/$PRODUCT_ID/acknowledge" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0" \
  -d "{\"external_user_id\": \"$USER_ID\"}")
ACK_HTTP_CODE=$(echo "$ACK_RESPONSE" | tail -n1)
ACK_RESP=$(echo "$ACK_RESPONSE" | sed '$d')

if [[ "$ACK_HTTP_CODE" != "200" ]]; then
    fail_test "acknowledge" "expected HTTP 200, got $ACK_HTTP_CODE"
fi

SUCCESS=$(echo "$ACK_RESP" | jq -r '.success')
if [[ "$SUCCESS" == "true" ]]; then
    echo -e "${GREEN} Acknowledged successfully${NC}"
else
    fail_test "acknowledge" "HTTP 200 response did not contain success=true"
fi

# 6. Verify Notification Flag (FALSE)
echo -e "${YELLOW}[5/5] Verifying Notification Flag = FALSE${NC}"
FINAL_STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0")
FINAL_STATUS_HTTP_CODE=$(echo "$FINAL_STATUS_RESPONSE" | tail -n1)
STATUS_RESP_FINAL=$(echo "$FINAL_STATUS_RESPONSE" | sed '$d')

if [[ "$FINAL_STATUS_HTTP_CODE" != "200" ]]; then
    fail_test "final_notification_status" "expected HTTP 200, got $FINAL_STATUS_HTTP_CODE"
fi

FLAG_FINAL=$(echo "$STATUS_RESP_FINAL" | jq -r '.payment_failure_notification')

if [[ "$FLAG_FINAL" == "false" || "$FLAG_FINAL" == "null" ]]; then
    echo -e "${GREEN} Notification cleared${NC}"
else
    fail_test "notification_flag_false" "expected false/null, got $FLAG_FINAL"
fi

# Report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "NOTIF-01",
  "test_name": "Payment Failure & Acknowledgment",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "webhook_http_code": $WEBHOOK_HTTP_CODE,
  "status_http_code": $STATUS_HTTP_CODE,
  "ack_http_code": $ACK_HTTP_CODE,
  "final_status_http_code": $FINAL_STATUS_HTTP_CODE
}
EOF
echo -e "${GREEN}✓ NOTIF-01 Test PASSED${NC}"
exit 0
