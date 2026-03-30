#!/bin/bash

##############################################################################
# SUB-03: Bridge User-Initiated Cancellation Test
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
WEBHOOK_ID="test-webhook-sub03-cancelled-$(date +%s)"

# Defaults
EMAIL="test-user@example.com"
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-03: Bridge User-Initiated Cancellation Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Ensure an active subscription exists
echo -e "${YELLOW}[1/4] Ensuring active subscription exists in Bridge DB${NC}"

SUB_QUERY="SELECT status, auto_renewing, current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND status = 'active' LIMIT 1;"
SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$SUB_RESULT" || "$SUB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${YELLOW}No active subscription found. Performing initial purchase (SUB-01 equivalent) first...${NC}"
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
    
    SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null || echo "")
fi

OLD_PERIOD_END=$(echo "$SUB_RESULT" | awk -F '|' '{print $3}' | tr -d '[:space:]')
echo -e "${GREEN}✓ Current Status: active, Period End: $OLD_PERIOD_END${NC}"
echo ""

# Step 3: Simulate Google Pub/Sub cancellation webhook
echo -e "${YELLOW}[2/4] Sending subscription.cancelled webhook${NC}"

TIMESTAMP=$(date +%s000)
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 3,
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

# Step 4: Verify status changed to cancelled and auto_renewing is false
echo -e "${YELLOW}[4/4] Verifying status and settings after cancellation${NC}"

NEW_SUB_QUERY="SELECT status, auto_renewing, current_period_end, cancellation_initiated_at FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
NEW_SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$NEW_SUB_QUERY" -t 2>/dev/null || echo "")

NEW_STATUS=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $1}' | tr -d '[:space:]')
NEW_AUTO_RENEWING=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $2}' | tr -d '[:space:]')
NEW_PERIOD_END=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $3}' | tr -d '[:space:]')
CANCELLATION_INITIATED=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $4}' | tr -d '[:space:]')

if [[ "$NEW_STATUS" != "cancelled" ]]; then
    echo -e "${RED}✗ Expected status 'cancelled', got '$NEW_STATUS'${NC}"
    exit 1
fi

if [[ "$NEW_AUTO_RENEWING" != "f" ]] && [[ "$NEW_AUTO_RENEWING" != "false" ]]; then
    echo -e "${RED}✗ Expected auto_renewing 'false', got '$NEW_AUTO_RENEWING'${NC}"
    exit 1
fi

if [[ -z "$CANCELLATION_INITIATED" ]]; then
    echo -e "${RED}✗ Expected cancellation_initiated_at to be set${NC}"
    exit 1
fi

if [[ "$NEW_PERIOD_END" != "$OLD_PERIOD_END" ]]; then
    echo -e "${RED}✗ Period end changed! Old: $OLD_PERIOD_END, New: $NEW_PERIOD_END${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Status correctly changed to 'cancelled'${NC}"
echo -e "${GREEN}✓ Auto Renewing: false${NC}"
echo -e "${GREEN}✓ Cancellation Initiated At: $CANCELLATION_INITIATED${NC}"
echo -e "${GREEN}✓ Period End Unchanged: $NEW_PERIOD_END (user retains access)${NC}"
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ SUB-03 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
