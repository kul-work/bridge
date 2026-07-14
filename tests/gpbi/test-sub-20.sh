#!/bin/bash

##############################################################################
# SUB-20: Price Change (Opt-In Increase, User Accepts)
# 
# Purpose: Verify that when a user accepts a price increase opt-in, the 
#          backend correctly records the renewal at the NEW price.
#
# Usage: ./test-sub-20.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN, BRIDGE_WEBHOOK_FUTURE_TS
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#   - jq installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: subscription.price_change_updated (notificationType 19) is logged.
#                      Subsequent renewal (notificationType 2) records payment with NEW price.
#                      Payment amount in pay.payments reflects the increased cents.
#                      Ensures pricing logic correctly adapts to user-accepted increases.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for SUB-20 snapshot assertions"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="sub-20-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-20-price_change-$TEST_RUN_ID"
REPORT_FILE="sub-20-report.json"
NEW_PRICE_CENTS=500

echo -e "${YELLOW}========================================${NC}"
echo "SUB-20: Price Change (Opt-In Increase)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_20_$TEST_RUN_ID"
echo -e "${GREEN}PASS: Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo ""

# Step 3: Establish active subscription
echo -e "${YELLOW}[1/5] Establishing active subscription${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-price-change-setup-20\"
  }" )

# Verify purchase
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

if [[ "$REGISTER_HTTP_CODE" != "200" || "$VERIFY_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}FAIL: Failed to establish active subscription (register=$REGISTER_HTTP_CODE, verify=$VERIFY_HTTP_CODE)${NC}"
    exit 1
fi

bridge_wait_for_db_glob \
    SETUP_STATE \
    "SELECT external_user_id || '|' || status FROM pay.subscriptions WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$DUMMY_TOKEN';" \
    "$USER_ID|active" \
    10 \
    1 || true

if [[ "$SETUP_STATE" != "$USER_ID|active" ]]; then
    echo -e "${RED}FAIL: Active subscription setup not visible in DB before webhook: $SETUP_STATE${NC}"
    exit 1
fi

echo -e "${GREEN}PASS: Active subscription established${NC}"
echo ""

# Step 4: Send price_change_updated webhook
echo -e "${YELLOW}[2/5] Sending subscription.price_change_updated webhook (notificationType 19)${NC}"
PRICE_CHANGE_EVENT_TIME_MS=$(($(date +%s) + 10))000

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$PRICE_CHANGE_EVENT_TIME_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 19,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"test-webhook-20-pc-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}PASS: Price change webhook sent${NC}"
echo ""

# Step 5: Simulate renewal with new price
echo -e "${YELLOW}[3/5] Simulating renewal with NEW price ($NEW_PRICE_CENTS cents)${NC}"
RENEWAL_EVENT_TIME_MS=$(($PRICE_CHANGE_EVENT_TIME_MS + 1000))

RENEWAL_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$RENEWAL_EVENT_TIME_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
RENEWAL_B64=$(echo -n "$RENEWAL_JSON" | base64 -w 0 2>/dev/null || echo -n "$RENEWAL_JSON" | base64)

# We use X-Test-Price-Cents to tell the mock provider what price to return
curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "X-Test-Price-Cents: $NEW_PRICE_CENTS" \
  -d "{
    \"message\": {
      \"data\": \"$RENEWAL_B64\",
      \"message_id\": \"test-webhook-20-renewal-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}PASS: Renewal webhook sent${NC}"
echo ""

# Step 6: Verify payment record with updated amount
echo -e "${YELLOW}[4/5] Verifying payment record with updated amount in Bridge DB${NC}"
export PGPASSWORD="postgres"
bridge_wait_for_db_glob \
    LATEST_PAYMENT \
    "SELECT amount_cents FROM pay.payments WHERE external_user_id = '$USER_ID' ORDER BY created_at DESC LIMIT 1;" \
    "$NEW_PRICE_CENTS" \
    10 \
    1 || true

if [[ "$LATEST_PAYMENT" == "$NEW_PRICE_CENTS" ]]; then
    echo -e "${GREEN}PASS: Latest payment amount is $LATEST_PAYMENT cents${NC}"
else
    echo -e "${RED}FAIL: Latest payment amount is $LATEST_PAYMENT cents, expected $NEW_PRICE_CENTS${NC}"
    exit 1
fi
echo ""

# Step 7: Verify app-facing subscription snapshot
echo -e "${YELLOW}[5/5] Verifying active post-renewal snapshot contract${NC}"
STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
STATUS_BODY=$(echo "$STATUS_RESPONSE" | sed '$d')

if [[ "$STATUS_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}FAIL: subscription-status returned HTTP $STATUS_HTTP_CODE${NC}"
    echo "$STATUS_BODY"
    exit 1
fi

if echo "$STATUS_BODY" | jq -e \
  '.is_premium == true and .status == "active" and .google_new_price_cents == null and .google_price_step_up_consent_deadline == null' > /dev/null; then
    echo -e "${GREEN}PASS: Snapshot shows active access with no pending price step-up fields${NC}"
else
    echo -e "${RED}FAIL: Post-renewal snapshot contract mismatch${NC}"
    echo "$STATUS_BODY" | jq .
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-20",
  "test_name": "Price Change (Opt-In Increase, User Accepts)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "price_change_verified": true,
  "snapshot_verified": true
}
EOF

echo -e "${GREEN}PASS: SUB-20 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
