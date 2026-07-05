#!/bin/bash

##############################################################################
# SUB-09: Subscription Revoked (Refund / Voided Purchase)
#
# Purpose: Verify revocation and refund flow via voided purchase webhook.
#
# Usage: ./test-sub-09.sh
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
#   Expected Behavior: Subscription status transitions to 'revoked'.
#                      'revoked_at' timestamp is correctly populated.
#                      The payment record in pay.payments is set to status='refunded'.
#                      Ensures revenue reversal and access revocation are synchronized.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for SUB-09 snapshot assertions"
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
TEST_RUN_ID="sub-09-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-09-token-$TEST_RUN_ID"
ORDER_ID="GPA.1234-5678-9012-SUB09"
REPORT_FILE="sub-09-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-09: Bridge Subscription Revoked (Refund)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/6] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo ""

# Step 3: Establish active subscription
echo -e "${YELLOW}[1/6] Establishing active subscription${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-09-setup\"
  }")

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
  }")

echo -e "${GREEN}✓ Initial active subscription established${NC}"
echo ""

# Step 4: Simulate Voided Purchase Webhook (Refund)
echo -e "${YELLOW}[2/6] Sending Voided Purchase Webhook (notificationType is not used here, it's a separate field)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "voidedPurchaseNotification": {
    "purchaseToken": "$DUMMY_TOKEN",
    "orderId": "$ORDER_ID",
    "productType": 1,
    "refundType": 0
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
      \"message_id\": \"test-webhook-09-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Voided purchase webhook sent${NC}"
echo ""

# Step 5: Verify status in DB
echo -e "${YELLOW}[3/6] Verifying 'revoked' state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=""
for attempt in $(seq 1 10); do
    RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT status, (revoked_at IS NOT NULL) as is_revoked_at_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')
    if [[ "$RES_DATA" == *"revoked"*"t"* ]]; then
        break
    fi
    sleep 1
done

# Expected: revoked | t
if [[ "$RES_DATA" == *"revoked"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'revoked' and revoked_at date is set${NC}"
else
    echo -e "${RED}✗ Failure: Revocation state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Step 6: Verify app-facing subscription snapshot
echo -e "${YELLOW}[4/6] Verifying revoked snapshot contract${NC}"
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
  '.is_premium == false and .status == "revoked" and .revocation_reason == "REFUND" and .revoked_at != null' > /dev/null; then
    echo -e "${GREEN}PASS: Snapshot shows revoked access with REFUND reason${NC}"
else
    echo -e "${RED}FAIL: Revoked snapshot contract mismatch${NC}"
    echo "$STATUS_BODY" | jq .
    exit 1
fi
echo ""

# Step 7: Verify payment status updated to 'refunded'
echo -e "${YELLOW}[5/6] Verifying payment status update to 'refunded'${NC}"
PAY_STATUS=""
for attempt in $(seq 1 10); do
    PAY_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]')
    if [[ "$PAY_STATUS" == "refunded" ]]; then
        break
    fi
    sleep 1
done

if [[ "$PAY_STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Success: Payment correctly marked as 'refunded'${NC}"
else
    echo -e "${RED}✗ Failure: Payment status is '$PAY_STATUS', expected 'refunded'${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-09",
  "test_name": "Bridge Subscription Revoked (Refund)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "snapshot_verified": true,
  "refund_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-09 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
