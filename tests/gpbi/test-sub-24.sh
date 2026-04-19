#!/bin/bash

##############################################################################
# SUB-24: Restart After Cancellation - Expiry Extension
# 
# Purpose: Verify that when a user re-enables auto-renew (RTDN Type 7),
#          the backend enriches with fresh data to extend future access.
#
# Usage: ./test-sub-24.sh
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
#   Expected Behavior: Subscription status transitions to 'active' in pay.subscriptions.
#                      current_period_end is extended to the future based on fresh provider data.
#                      auto_renewing flag is set to true.
#                      Ensures users who change their mind about cancellation regain full future access.
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
DUMMY_TOKEN="test-sub-24-restart-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-24-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-24: Restart After Cancellation"
echo -e "${YELLOW}========================================${NC}"
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

# Step 3: Establish cancelled subscription with PAST expiry
echo -e "${YELLOW}[1/5] Establishing cancelled subscription with PAST expiry${NC}"

# Seed the database directly for this scenario
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, current_period_end) 
      VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'cancelled', false, '$DUMMY_TOKEN', NOW() - INTERVAL '2 days');" > /dev/null

echo -e "${GREEN}✓ Cancelled subscription seeded in DB (Expiry: 2 days ago)${NC}"
echo ""

# Step 4: Send RTDN Type 7 (SUBSCRIPTION_RESTARTED) webhook
echo -e "${YELLOW}[2/5] Sending RTDN Type 7 (SUBSCRIPTION_RESTARTED) webhook${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 7,
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
      \"message_id\": \"test-webhook-24-restart-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Restart webhook sent${NC}"
echo ""

# Step 5: Verify subscription state in DB
echo -e "${YELLOW}[3/5] Verifying subscription enrichment in Bridge DB${NC}"
export PGPASSWORD="postgres"
SUB_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, auto_renewing, (current_period_end > NOW()) as in_future FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: active | t | t
if [[ "$SUB_DATA" == *"active"*"t"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is active, auto-renew enabled, and expiry extended to FUTURE${NC}"
else
    echo -e "${RED}✗ Failure: Subscription enrichment failed: $SUB_DATA${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-24",
  "test_name": "Restart After Cancellation",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "subscription_data": "$SUB_DATA"
}
EOF

echo -e "${GREEN}✓ SUB-24 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
