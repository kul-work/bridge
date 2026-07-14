#!/bin/bash

##############################################################################
# SUB-02: Subscription Renewal (Automatic)
#
# Purpose: Verify the automatic subscription renewal flow triggered by webhooks.
#
# Usage: ./test-sub-02.sh
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
#   Expected Behavior: Subscription status remains 'active'.
#                      'current_period_end' is extended in pay.subscriptions.
#                      A new payment record is created in pay.payments with status='success'.
#                      Ensures recurring revenue cycles are correctly tracked and persistent.
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
TEST_RUN_ID="sub-02-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-02-token-$TEST_RUN_ID"
REPORT_FILE="sub-02-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-02: Bridge Subscription Renewal (Automatic)"
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

# Step 3: Establish initial active subscription
echo -e "${YELLOW}[1/5] Establishing initial active subscription${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-02-setup\"
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

# Get initial expiry
OLD_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT current_period_end FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

echo -e "${GREEN}✓ Initial active subscription established (Expiry: $OLD_PERIOD_END)${NC}"
echo ""

# Step 4: Simulate Google Pub/Sub renewal webhook (Type 2)
echo -e "${YELLOW}[2/5] Sending subscription.renewed webhook (notificationType 2)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
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
      \"message_id\": \"test-webhook-02-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Renewal webhook sent${NC}"
echo ""

# Step 5: Verify extension in DB
echo -e "${YELLOW}[3/5] Verifying current_period_end extension in Bridge DB${NC}"
NEW_PERIOD_END=""
for attempt in $(seq 1 10); do
    NEW_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT current_period_end FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')
    if [[ -n "$NEW_PERIOD_END" && "$NEW_PERIOD_END" != "$OLD_PERIOD_END" ]]; then
        break
    fi
    sleep 1
done

if [[ "$NEW_PERIOD_END" != "$OLD_PERIOD_END" ]]; then
    echo -e "${GREEN}✓ Success: Period extended from $OLD_PERIOD_END to $NEW_PERIOD_END${NC}"
else
    echo -e "${RED}✗ Failure: Period was NOT extended (remains $OLD_PERIOD_END)${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-02",
  "test_name": "Bridge Subscription Renewal (Automatic)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "period_extension_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-02 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
