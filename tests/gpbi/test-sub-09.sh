#!/bin/bash

##############################################################################
# SUB-09: Subscription Revoked (Refund / Voided Purchase)
#
# Purpose: Verify revocation and refund flow via voided purchase webhook.
#
# Usage: ./test-sub-09.sh
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
#   Expected Behavior: Subscription status transitions to 'revoked'.
#                      'revoked_at' timestamp is correctly populated.
#                      The payment record in pay.payments is set to status='refunded'.
#                      Ensures revenue reversal and access revocation are synchronized.
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
DUMMY_TOKEN="test-sub-09-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
ORDER_ID="GPA.1234-5678-9012-SUB09"
REPORT_FILE="sub-09-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-09: Bridge Subscription Revoked (Refund)"
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
    \"reason\": \"test-sub-09-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-09-$(date +%s)\"
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

echo -e "${GREEN}✓ Initial active subscription established${NC}"
echo ""

# Step 4: Simulate Voided Purchase Webhook (Refund)
echo -e "${YELLOW}[2/5] Sending Voided Purchase Webhook (notificationType is not used here, it's a separate field)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$BRIDGE_WEBHOOK_FUTURE_TS",
  "voidedPurchaseNotification": {
    "purchaseToken": "$DUMMY_TOKEN",
    "orderId": "$ORDER_ID",
    "productType": 1,
    "refundType": 0
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
      \"message_id\": \"test-webhook-09-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Voided purchase webhook sent${NC}"
echo ""

# Step 5: Verify status in DB
echo -e "${YELLOW}[3/5] Verifying 'revoked' state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, (revoked_at IS NOT NULL) as is_revoked_at_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: revoked | t
if [[ "$RES_DATA" == *"revoked"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'revoked' and revoked_at date is set${NC}"
else
    echo -e "${RED}✗ Failure: Revocation state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Step 6: Verify payment status updated to 'refunded'
echo -e "${YELLOW}[4/5] Verifying payment status update to 'refunded'${NC}"
PAY_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]')

if [[ "$PAY_STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Success: Payment correctly marked as 'refunded'${NC}"
else
    echo -e "${RED}✗ Failure: Payment status is '$PAY_STATUS', expected 'refunded'${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-09",
  "test_name": "Bridge Subscription Revoked (Refund)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "refund_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-09 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
