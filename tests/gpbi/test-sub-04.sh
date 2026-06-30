#!/bin/bash

##############################################################################
# SUB-04: Grace Period Entry and Recovery
# 
# Purpose: Verify the subscription grace period lifecycle via webhooks.
#
# Usage: ./test-sub-04.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN, BRIDGE_WEBHOOK_FUTURE_TS
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Subscription transitions: active -> past_due (6) -> active (1).
#                      'google_grace_period_start' is set during grace entry.
#                      'google_grace_period_start' is cleared upon recovery.
#                      Ensures seamless access transition during payment failure cycles.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="sub-04-${TIMESTAMP}-$$"
DUMMY_TOKEN="test-sub-04-token-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-04-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-04: Grace Period Entry and Recovery"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
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
    \"reason\": \"test-sub-04-setup\"
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

echo -e "${GREEN}✓ Active subscription established${NC}"
echo ""

# Step 4: Simulate Grace Period Entry (Type 6)
echo -e "${YELLOW}[2/5] Sending Grace Period Entry Webhook (Type 6)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 6,
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
      \"message_id\": \"test-webhook-04-grace-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Grace period entry webhook sent${NC}"
echo ""

# Step 5: Verify status in DB
echo -e "${YELLOW}[3/5] Verifying 'past_due' (grace period) state in Bridge DB${NC}"
export PGPASSWORD="postgres"
bridge_wait_for_db_glob \
    RES_DATA \
    "SELECT status, (google_grace_period_start IS NOT NULL) as grace_start_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" \
    "*past_due*t*" \
    10 \
    1 || true

# Expected: past_due | t
if [[ "$RES_DATA" == *"past_due"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'past_due' and grace start date is set${NC}"
else
    echo -e "${RED}✗ Failure: Grace period state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Step 6: Simulate Recovery (Type 1)
echo -e "${YELLOW}[4/5] Sending Recovery Webhook (Type 1)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(($BRIDGE_WEBHOOK_FUTURE_TS + 1000))",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 1,
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
      \"message_id\": \"test-webhook-04-recover-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Recovery webhook sent${NC}"
echo ""

# Step 7: Final Validation
echo -e "${YELLOW}[5/5] Verifying 'active' state after recovery in Bridge DB${NC}"
export PGPASSWORD="postgres"
bridge_wait_for_db_glob \
    RES_DATA \
    "SELECT status, (google_grace_period_start IS NULL) as grace_start_cleared FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" \
    "*active*t*" \
    10 \
    1 || true

# Expected: active | t
if [[ "$RES_DATA" == *"active"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'active' and grace period fields are cleared${NC}"
else
    echo -e "${RED}✗ Failure: Recovery state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-04",
  "test_name": "Grace Period Entry and Recovery",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "grace_period_entry_verified": true,
  "recovery_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-04 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
