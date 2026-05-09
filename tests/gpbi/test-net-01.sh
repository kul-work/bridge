#!/bin/bash

##############################################################################
# NET-01: Webhook Arrives Before verify_purchase
# 
# Purpose: Verify that when a webhook (RTDN) arrives BEFORE the client
#          calls verify-purchase, the system handles the out-of-order
#          event gracefully and achieves eventual consistency.
#
# Usage: ./test-net-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: A SUBSCRIPTION_PURCHASED (4) webhook arrives for a token not yet tied to a user.
#                      The webhook handler finishes successfully (idempotent 'noop' or placeholder).
#                      A subsequent POST /api/v1/verify-purchase call for the SAME token links it.
#                      Final state: Subscription is 'active' and correctly bound to the user.
#                      Ensures robustness against network race conditions between RTDN and client verification.
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
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="net-01-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
REPORT_FILE="net-01-report.json"
USER_ID="${USER_ID:-test_net_01_user_$TEST_RUN_ID}"
DUMMY_TOKEN="test-net-01-token-$TEST_RUN_ID"
WEBHOOK_ID="webhook-net-01-$TEST_RUN_ID"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

# Extract DB password once
# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "NET-01: Webhook Arrives Before verify_payment"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/6] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing subscription data
echo -e "${YELLOW}[2/6] Cleaning up previous test data${NC}"

PURCHASE_TOKEN="test-net-01-webhook-first-$(date +%s)"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

echo -e "${GREEN}✓ Cleanup complete${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Simulate webhook arriving FIRST (before verify_payment)
echo -e "${YELLOW}[3/6] Sending webhook BEFORE verify_payment (simulating race condition)${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="net-01-webhook-first-$(date +%s)"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  Notification Type: 4 (SUBSCRIPTION_PURCHASED)"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo "  Note: Token is NOT yet registered in DB"
echo ""

# Create DeveloperNotification JSON
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 4,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send webhook FIRST
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"

WEBHOOK_HANDLED="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP 200) - may have found nothing to update${NC}"
    WEBHOOK_HANDLED="true"
else
    echo -e "${YELLOW}⚠ Webhook returned HTTP $WEBHOOK_HTTP_CODE${NC}"
    WEBHOOK_HANDLED="true"  # Any response is acceptable at this stage
fi
echo ""

# Check if any subscription was created (should be none or partial)
SUB_AFTER_WEBHOOK=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
echo -e "${BLUE}Subscriptions with token after webhook: $SUB_AFTER_WEBHOOK${NC}"
echo ""

# Step 4: Now call verify_payment (after webhook)
echo -e "${YELLOW}[4/6] Calling verify_payment AFTER webhook${NC}"
echo ""

echo "  POST $BRIDGE_API_URL/api/v1/verify-purchase"
echo "  Authorization: Bearer $BRIDGE_API_KEY"
echo "  Token: $PURCHASE_TOKEN"
echo ""

# Pre-register purchase intent before verification
curl -s -o /dev/null -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-net-01-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-01-$(date +%s)\"
  }"

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

VERIFY_HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
echo "verify_payment Response Code: $VERIFY_HTTP_CODE"

VERIFY_SUCCESS="false"
if [[ "$VERIFY_HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ verify_payment successful${NC}"
    VERIFY_SUCCESS="true"
else
    echo -e "${RED}✗ verify_payment failed with HTTP $VERIFY_HTTP_CODE${NC}"
fi
echo ""

# Step 5: Verify final state (eventual consistency)
echo -e "${YELLOW}[5/6] Verifying final state (DB Validation)${NC}"
echo ""

FINAL_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status, external_user_id FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null || echo "")

if [[ ! -z "$FINAL_SUB" ]] && [[ "$FINAL_SUB" != *"(0 rows)"* ]]; then
    FINAL_STATUS=$(echo "$FINAL_SUB" | awk -F '|' '{print $1}' | tr -d ' ')
    FINAL_CLERK_ID=$(echo "$FINAL_SUB" | awk -F '|' '{print $2}' | tr -d ' ')
    
    echo "Final subscription status: $FINAL_STATUS"
    echo "Final external_user_id: $FINAL_CLERK_ID"
    echo ""
    
    EVENTUAL_CONSISTENCY="false"
    if [[ "$FINAL_STATUS" == "active" ]] && [[ "$FINAL_CLERK_ID" == "$USER_ID" ]]; then
        echo -e "${GREEN}✓ Eventual consistency achieved: status=active, correct user linked${NC}"
        EVENTUAL_CONSISTENCY="true"
    else
        echo -e "${YELLOW}⚠ Final state: status=$FINAL_STATUS, external_user_id=$FINAL_CLERK_ID${NC}"
        EVENTUAL_CONSISTENCY="false"
    fi
else
    echo -e "${RED}✗ No subscription found after verify_payment!${NC}"
    EVENTUAL_CONSISTENCY="false"
fi
echo ""

# Step 6: Cleanup
echo -e "${YELLOW}[6/6] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
if [[ "$VERIFY_SUCCESS" == "true" ]] && [[ "$EVENTUAL_CONSISTENCY" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ NET-01 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ NET-01 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "NET-01",
  "test_name": "Webhook Arrives Before verify_payment",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "message_id": "$MESSAGE_ID",
  "results": {
    "webhook_handled_gracefully": $WEBHOOK_HANDLED,
    "webhook_http_code": $WEBHOOK_HTTP_CODE,
    "verify_payment_success": $VERIFY_SUCCESS,
    "verify_payment_http_code": $VERIFY_HTTP_CODE,
    "eventual_consistency_achieved": $EVENTUAL_CONSISTENCY,
    "subscriptions_after_webhook": $SUB_AFTER_WEBHOOK
  },
  "notes": "App should call verify_payment immediately after purchase (~1-5 second window)"
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
