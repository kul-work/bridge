#!/bin/bash

##############################################################################
# SUB-05: Subscription Expiration Lifecycle
#
# Purpose: Verify final subscription expiration triggered by webhooks.
#
# Usage: ./test-sub-05.sh
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
#   Expected Behavior: Subscription status transitions to 'expired'.
#                      Access is revoked (status indicates final expiration).
#                      Historical payment records remain unchanged.
#                      Validates webhook mapping of notificationType 13 (EXPIRED).
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
TEST_RUN_ID="sub-05-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-05-token-$TEST_RUN_ID"
REPORT_FILE="sub-05-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-05: Bridge Subscription Expiration"
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

# Step 3: Establish cancelled subscription
echo -e "${YELLOW}[1/5] Establishing cancelled subscription${NC}"

# Seed cancelled subscription
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, status, auto_renewing, purchase_token) 
      VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'cancelled', false, '$DUMMY_TOKEN');" > /dev/null

echo -e "${GREEN}✓ Cancelled subscription seeded in DB${NC}"
echo ""

# Step 4: Simulate Google Pub/Sub expiration webhook
echo -e "${YELLOW}[2/5] Sending subscription.expired webhook (notificationType 13)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 13,
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
      \"message_id\": \"test-webhook-05-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Expiration webhook sent${NC}"
echo ""

# Step 5: Verify status changed to expired
echo -e "${YELLOW}[3/5] Verifying status after expiration${NC}"
export PGPASSWORD="postgres"
bridge_wait_for_db_glob \
    STATUS \
    "SELECT status FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" \
    "expired" \
    10 \
    1 || true

if [[ "$STATUS" == "expired" ]]; then
    echo -e "${GREEN}✓ Success: Status is '$STATUS'${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected 'expired'${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-05",
  "test_name": "Bridge Subscription Expiration",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": 0,
  "verify_http_code": 0,
  "expiration_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-05 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
