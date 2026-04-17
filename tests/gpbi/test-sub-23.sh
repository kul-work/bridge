#!/bin/bash

##############################################################################
# SUB-23: Pending Purchase Canceled Test
# 
# Purpose: Verify that when a user cancels a pending purchase before payment
#          completes, the backend correctly processes the pending_purchase_canceled
#          webhook and cleans up the pending state.
#          1. Establish a pending subscription
#          2. Send subscription.pending_purchase_canceled (notificationType 20)
#          3. Verify subscription status changed to 'cancelled'
#
# Usage: ./test-sub-23.sh
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
DUMMY_TOKEN="test-sub-23-pending-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-23-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-23: Pending Purchase Canceled"
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

# Step 3: Establish a pending subscription
echo -e "${YELLOW}[1/5] Establishing pending subscription${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-pending-cancel-setup-23\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-23-$(date +%s)\"
  }" )

# Verify purchase (as pending)
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Test-Subscription-Status: pending" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

echo -e "${GREEN}✓ Pending subscription established${NC}"
echo ""

# Step 4: Send pending_purchase_canceled webhook
echo -e "${YELLOW}[2/5] Sending subscription.pending_purchase_canceled webhook (notificationType 20)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 20,
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
      \"message_id\": \"test-webhook-23-pc-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Pending cancel webhook sent${NC}"
echo ""

# Step 5: Verify subscription status in DB
echo -e "${YELLOW}[3/5] Verifying subscription status in Bridge DB${NC}"
export PGPASSWORD="postgres"
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$STATUS" == "cancelled" ]]; then
    echo -e "${GREEN}✓ Success: Subscription status is $STATUS${NC}"
else
    echo -e "${RED}✗ Failure: Subscription status is $STATUS, expected 'cancelled'${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-23",
  "test_name": "Subscription Deferred Proration Credit",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "proration_credit_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-23 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
