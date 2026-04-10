#!/bin/bash

##############################################################################
# SUB-03: Bridge User-Initiated Cancellation Test
#
# Purpose: Verify the user-initiated subscription cancellation flow:
#          1. Establish an active subscription
#          2. Simulate Google Pub/Sub cancellation webhook (notificationType 3)
#          3. Verify status changed to "cancelled" in Bridge DB
#          4. Verify auto_renewing is set to false
#          5. Verify cancellation_initiated_at is populated
#
# Usage: ./test-sub-03.sh
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
DUMMY_TOKEN="test-sub-03-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-03: Bridge User-Initiated Cancellation"
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

# Step 3: Establish active subscription
echo -e "${YELLOW}[1/5] Establishing active subscription${NC}"

# Pre-register purchase
curl -s -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-03-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-03-$(date +%s)\"
  }" > /dev/null

# Verify purchase
curl -s -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

echo -e "${GREEN}✓ Active subscription established${NC}"
echo ""

# Step 4: Simulate Google Pub/Sub cancellation webhook
echo -e "${YELLOW}[2/5] Sending subscription.cancelled webhook (notificationType 3)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 3,
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
      \"message_id\": \"test-webhook-03-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Cancellation webhook sent${NC}"
echo ""

# Step 5: Verify status and settings in DB
echo -e "${YELLOW}[3/5] Verifying status and settings after cancellation${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, auto_renewing, (cancellation_initiated_at IS NOT NULL) as cancelled_at_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: cancelled | f | t
if [[ "$RES_DATA" == *"cancelled"*"f"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'cancelled', auto-renew set to false, and cancellation date is set${NC}"
else
    echo -e "${RED}✗ Failure: Cancellation state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

echo -e "${GREEN}✓ SUB-03 Bridge Test PASSED${NC}"
exit 0
