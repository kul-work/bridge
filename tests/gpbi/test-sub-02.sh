#!/bin/bash

##############################################################################
# SUB-02: Bridge Subscription Renewal (Automatic) Test
#
# Purpose: Verify the automatic subscription renewal flow:
#          1. Ensure an active subscription exists (or create it)
#          2. Simulate Google Pub/Sub renewal webhook (notificationType: 2)
#          3. Wait for async processing
#          4. Verify current_period_end was extended in Bridge DB
#
# Usage: ./test-sub-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * PRODUCT_ID_SUB, PROVIDER, PACKAGE_NAME
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#     * WEBHOOK_INGRESS_TOKEN
#   - psql installed and in PATH
#   - Active subscription exists (script will create if missing)
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
DUMMY_TOKEN="test-subscription-sub01-12345-active"
PRODUCT_ID="$PRODUCT_ID_SUB"
WEBHOOK_ID="test-webhook-sub02-renewal-$(date +%s)"

# Note: Using BRIDGE_API_URL from globals.cfg

echo -e "${YELLOW}========================================${NC}"
echo "SUB-02: Bridge Subscription Renewal (Automatic) Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
# For this test, we use a fixed test user ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/4] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null || true
echo -e "${GREEN}✓ Previous test data removed${NC}"
echo ""

# Step 3: Ensure an active subscription exists (Mocking it for this test if needed)
echo -e "${YELLOW}[1/4] Ensuring active subscription exists in Bridge DB${NC}"

# Ensure user and app have some initial state or just run this test after SUB-01
# For a robust test, we can manually insert or run SUB-01 logic.
# I'll just check if it's there.

export PGPASSWORD="postgres"
SUB_QUERY="SELECT current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND status = 'active' LIMIT 1;"
OLD_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')

if [[ -z "$OLD_PERIOD_END" ]]; then
    echo -e "${YELLOW}No active subscription found. Performing initial purchase (SUB-01 equivalent) first...${NC}"
    # Pre-register purchase
    curl -s -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      -d "{
        \"external_user_id\": \"$USER_ID\",
        \"provider\": \"$PROVIDER\",
        \"subscription_id\": \"$PRODUCT_ID\",
        \"reason\": \"test-registration\",
        \"product_type\": \"subscription\",
        \"amount_cents\": 0,
        \"transaction_id\": \"test-reg-$(date +%s)\"
      }" > /dev/null
    
    # Verify purchase
    VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      -d "{
        \"external_user_id\": \"$USER_ID\",
        \"provider\": \"$PROVIDER\",
        \"subscription_id\": \"$PRODUCT_ID\",
        \"purchase_token\": \"$DUMMY_TOKEN\",
        \"product_type\": \"subscription\"
      }")
    VERIFY_HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
    VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | head -n -1)
    echo "Verify-purchase HTTP: $VERIFY_HTTP_CODE"
    echo "Verify-purchase Response: $VERIFY_BODY"
    
    OLD_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')
fi

echo -e "${GREEN}✓ Current Period End: $OLD_PERIOD_END${NC}"
echo ""

# Step 4: Simulate Google Pub/Sub renewal webhook
echo -e "${YELLOW}[2/4] Sending subscription.paid webhook (Renewal)${NC}"

TIMESTAMP=$(date +%s000)
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$WEBHOOK_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/subscriptions/test-sub\"
  }")

WH_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
echo "Webhook Response Code: $WH_HTTP_CODE"

if [[ "$WH_HTTP_CODE" != "200" ]] && [[ "$WH_HTTP_CODE" != "204" ]]; then
    echo -e "${RED}✗ Webhook failed with HTTP $WH_HTTP_CODE${NC}"
    exit 1
fi

echo -e "${YELLOW}[3/4] Waiting for async processing...${NC}"
sleep 2

# Step 4: Verify extension
echo -e "${YELLOW}[4/4] Verifying current_period_end was extended${NC}"

NEW_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')

if [[ "$NEW_PERIOD_END" == "$OLD_PERIOD_END" ]]; then
    echo -e "${RED}✗ Subscription period was NOT extended!${NC}"
    echo "  Old: $OLD_PERIOD_END"
    echo "  New: $NEW_PERIOD_END"
    exit 1
fi

echo -e "${GREEN}✓ Period extended successfully!${NC}"
echo "  Old: $OLD_PERIOD_END"
echo "  New: $NEW_PERIOD_END"
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ SUB-02 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
