#!/bin/bash

##############################################################################
# SUB-04: Grace Period Entry and Recovery Test
# 
# Purpose: Verify the grace period entry and recovery flow:
#          1. Establish an active subscription
#          2. Simulate Google Pub/Sub grace period webhook (notificationType 6)
#          3. Verify status changed to "in_grace_period" and fields set
#          4. Simulate recovery webhook (notificationType 1)
#          5. Verify status returned to "active" and grace fields cleared
#
# Usage: ./test-sub-04.sh
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
DUMMY_TOKEN="test-sub-04-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-04-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-04: Grace Period Entry and Recovery"
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
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-04-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-04-$(date +%s)\"
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
      \"message_id\": \"test-webhook-04-grace-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Grace period entry webhook sent${NC}"
echo ""

# Step 5: Verify status in DB
echo -e "${YELLOW}[3/5] Verifying 'past_due' (grace period) state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, (google_grace_period_start IS NOT NULL) as grace_start_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

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
      \"message_id\": \"test-webhook-04-recover-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Recovery webhook sent${NC}"
echo ""

# Step 7: Final Validation
echo -e "${YELLOW}[5/5] Verifying 'active' state after recovery in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, (google_grace_period_start IS NULL) as grace_start_cleared FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: active | t
if [[ "$RES_DATA" == *"active"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'active' and grace period fields are cleared${NC}"
else
    echo -e "${RED}✗ Failure: Recovery state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-04",
  "test_name": "Grace Period Entry and Recovery",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
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
