#!/bin/bash

##############################################################################
# SUB-05: Bridge Subscription Expiration Test
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
DUMMY_TOKEN="test-subscription-sub01-12345"
PRODUCT_ID="$PRODUCT_ID_SUB"
WEBHOOK_ID="test-webhook-sub05-expired-$(date +%s)"

# Defaults
EMAIL="test-user@example.com"
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-05: Bridge Subscription Expiration Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Ensure a cancelled subscription exists
echo -e "${YELLOW}[1/4] Ensuring cancelled subscription exists in Bridge DB${NC}"

SUB_QUERY="SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' LIMIT 1;"
OLD_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')

if [[ "$OLD_STATUS" != "cancelled" ]]; then
    echo -e "${YELLOW}Status is '$OLD_STATUS', not 'cancelled'. Running SUB-03 equivalent...${NC}"
    # Minimal cancellation simulation
    # (In Bridge, cancellation is also a webhook or API call)
    # We'll just send the cancellation webhook.
    TIMESTAMP=$(date +%s000)
    CANC_JSON="{\"message\":{\"data\":\"$(echo -n "{\"version\":\"1.0\",\"packageName\":\"$PACKAGE_NAME\",\"eventTimeMillis\":\"$TIMESTAMP\",\"subscriptionNotification\":{\"version\":\"1.0\",\"notificationType\":3,\"purchaseToken\":\"$DUMMY_TOKEN\",\"subscriptionId\":\"$PRODUCT_ID\"}}" | base64 -w 0)\",\"message_id\":\"test-canc-$(date +%s)\",\"attributes\":{}},\"subscription\":\"projects/test-project/subscriptions/test-sub\"}"
    
    curl -s -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-token" \
      -d "$CANC_JSON" > /dev/null
    
    sleep 1
    OLD_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')
fi

echo -e "${GREEN}✓ Current Status: $OLD_STATUS${NC}"
echo ""

# Step 3: Simulate Google Pub/Sub expiration webhook
echo -e "${YELLOW}[2/4] Sending subscription.expired webhook${NC}"

TIMESTAMP=$(date +%s000)
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 13,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode
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

echo -e "${YELLOW}[3/4] Waiting for async processing...${NC}"
sleep 2

# Step 4: Verify status changed to expired
echo -e "${YELLOW}[4/4] Verifying status after expiration${NC}"

NEW_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')

if [[ "$NEW_STATUS" != "expired" ]]; then
    echo -e "${RED}✗ Expected status 'expired', got '$NEW_STATUS'${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Status correctly changed to 'expired'${NC}"
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ SUB-05 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
