#!/bin/bash

##############################################################################
# SUB-24: Restart After Cancellation - Expiry Extension Test
# 
# Purpose: Verify that when a user re-enables auto-renew (RTDN Type 7),
#          the backend enriches with fresh Google Play API data so the
#          expiry date is updated to the future (not left in the past).
#          1. Establish a cancelled subscription with PAST expiry
#          2. Send RTDN Type 7 (SUBSCRIPTION_RESTARTED) webhook
#          3. Verify status changed to 'active'
#          4. Verify current_period_end is now in the FUTURE
#
# Usage: ./test-sub-24.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * PRODUCT_ID_SUB, PROVIDER, PACKAGE_NAME
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
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
  -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, current_period_end) 
      VALUES ('$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'cancelled', false, '$DUMMY_TOKEN', NOW() - INTERVAL '2 days');" > /dev/null

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

echo -e "${GREEN}✓ SUB-24 Bridge Test PASSED${NC}"
exit 0
