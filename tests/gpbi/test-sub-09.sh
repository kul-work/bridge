#!/bin/bash

##############################################################################
# SUB-09: Bridge Subscription Revoked (Refund) Test
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
DUMMY_TOKEN="test-subscription-sub09-$(date +%s)"
PRODUCT_ID="$PRODUCT_ID_SUB"
ORDER_ID="GPA.1234-5678-9012-SUB09"
WEBHOOK_ID="wh-sub09-void-$(date +%s)"

# Defaults
EMAIL="test-user@example.com"
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-09: Bridge Subscription Revoked (Refund) Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_09"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Initial Active Purchase
echo -e "${YELLOW}[1/4] Initial Purchase with ACTIVE status${NC}"

# Clean up
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

# Verify purchase
curl -s -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

# Verify status in DB
SUB_QUERY="SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
DB_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t | tr -d ' ')

if [[ "$DB_STATUS" == "active" ]] || [[ "$DB_STATUS" == "trial" ]]; then
    echo -e "${GREEN}✓ Initial status is '$DB_STATUS'${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$DB_STATUS' (Expected active/trial)${NC}"
    exit 1
fi

# Step 3: Simulate Voided Purchase Webhook
echo -e "${YELLOW}[2/4] Sending Voided Purchase Webhook (Refund)${NC}"
TIMESTAMP=$(date +%s000)

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "voidedPurchaseNotification": {
    "purchaseToken": "$DUMMY_TOKEN",
    "orderId": "$ORDER_ID",
    "productType": 1,
    "refundType": 0
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
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

echo "Waiting for processing..."
sleep 2

# Step 4: Verify Status and Revocation Reason
echo -e "${YELLOW}[3/4] Verifying status and revocation details${NC}"

EXTENDED_SUB_QUERY="SELECT status, revoked_at, revocation_reason FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$EXTENDED_SUB_QUERY" -t 2>/dev/null || echo "")

NEW_STATUS=$(echo "$SUB_RESULT" | awk -F '|' '{print $1}' | tr -d '[:space:]')
REVOKED_AT=$(echo "$SUB_RESULT" | awk -F '|' '{print $2}' | tr -d '[:space:]')
REVOCATION_REASON=$(echo "$SUB_RESULT" | awk -F '|' '{print $3}' | tr -d '[:space:]')

if [[ "$NEW_STATUS" != "revoked" ]]; then
    echo -e "${RED}✗ Expected status 'revoked', got '$NEW_STATUS'${NC}"
    exit 1
fi

if [[ -z "$REVOKED_AT" ]]; then
    echo -e "${RED}✗ Expected revoked_at to be set${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Status correctly changed to 'revoked'${NC}"
echo -e "${GREEN}✓ Revocation Reason: $REVOCATION_REASON${NC}"
echo -e "${GREEN}✓ Revoked At: $REVOKED_AT${NC}"

# Step 5: Verify Payment Status
echo -e "${YELLOW}[4/4] Verifying payment status update to 'refunded'${NC}"

PAYMENT_QUERY="SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"
PAYMENT_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$PAYMENT_QUERY" -t | tr -d ' ')

if [[ "$PAYMENT_STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Payment status correctly changed to 'refunded'${NC}"
else
    echo -e "${RED}✗ Expected payment status 'refunded', got '$PAYMENT_STATUS'${NC}"
    exit 1
fi

echo ""
echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ SUB-09 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
