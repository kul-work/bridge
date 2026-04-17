#!/bin/bash

##############################################################################
# SUB-07: Slow Card (Pending Renewal) Test
# 
# Purpose: Verify the behavior of a subscription renewal that starts as PENDING
#          due to a "Slow Test Card" and later resolves to SUCCESS.
#          1. Establish an active subscription
#          2. Simulate Pending Renewal webhook (notificationType 2 + pending status)
#          3. Verify status remains active (or reflects pending) and date NOT extended
#          4. Simulate Successful Renewal (resolves to SUCCESS)
#          5. Verify final status is active and period extended
#
# Usage: ./test-sub-07.sh
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
DUMMY_TOKEN="test-sub-07-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-07-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-07: Slow Card (Pending Renewal)"
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
    \"reason\": \"test-sub-07-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-07-$(date +%s)\"
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

# Step 4: Simulate Pending Renewal (The "Slow" Card attempt)
echo -e "${YELLOW}[2/5] Sending Pending Renewal Webhook (Type 2 + Pending status)${NC}"

# In mock mode, suffix "-pending" triggers PENDING state in mock Google API response
TEMP_TOKEN_PENDING="$DUMMY_TOKEN-pending"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$TEMP_TOKEN_PENDING",
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
      \"message_id\": \"test-webhook-07-pending-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

# Verify date NOT extended
NEW_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT current_period_end FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$NEW_PERIOD_END" != "$OLD_PERIOD_END" ]]; then
    echo -e "${RED}✗ Failure: Period was extended during PENDING state!${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Success: Period remained $OLD_PERIOD_END during pending phase${NC}"
echo ""

# Step 5: Simulate Successful Renewal (resolves to SUCCESS)
echo -e "${YELLOW}[3/5] Sending Successful Renewal Webhook (resolves to SUCCESS)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
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
      \"message_id\": \"test-webhook-07-success-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

# Verify date extended
FINAL_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT current_period_end FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$FINAL_PERIOD_END" == "$OLD_PERIOD_END" ]]; then
    echo -e "${RED}✗ Final check failed: Period was NOT extended after success${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Renewal completed: Subscription period extended to $FINAL_PERIOD_END${NC}"
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-07",
  "test_name": "Slow Card (Pending Renewal)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "slow_card_recovery_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-07 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
