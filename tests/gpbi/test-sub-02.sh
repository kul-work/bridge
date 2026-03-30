#!/bin/bash

##############################################################################
# SUB-02: Bridge Subscription Renewal (Automatic) Test
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
WEBHOOK_ID="test-webhook-sub02-renewal-$(date +%s)"

# Defaults
EMAIL="test-user@example.com"
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-02: Bridge Subscription Renewal (Automatic) Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID (should be the same as SUB-01 for renewal)
# Normally you'd pass this via --email but for Bridge we use external_user_id directly.
# Since we just ran SUB-01, we might need a consistent user_id across tests if we are running in sequence.
# For simplicity, I'll allow passing USER_ID as an env var or default to the last one if we can find it.
# Actually, I'll just use the same logic as the original: fetch it.
# But Bridge doesn't have a 'users' table with email by default (it's opaque).
# Wait, check AGENTS.md: "Avoid recording general user PII (emails, names) unnecessarily in Bridge DB."
# So I'll just use a test user id: "test_sub_user_01"

USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Ensure an active subscription exists (Mocking it for this test if needed)
echo -e "${YELLOW}[1/4] Ensuring active subscription exists in Bridge DB${NC}"

# Ensure user and app have some initial state or just run this test after SUB-01
# For a robust test, we can manually insert or run SUB-01 logic.
# I'll just check if it's there.

SUB_QUERY="SELECT current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND status = 'active' LIMIT 1;"
OLD_PERIOD_END=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')

if [[ -z "$OLD_PERIOD_END" ]]; then
    echo -e "${YELLOW}No active subscription found. Performing initial purchase (SUB-01 equivalent) first...${NC}"
    # Minimal verify-purchase call
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
    
    OLD_PERIOD_END=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')
fi

echo -e "${GREEN}✓ Current Period End: $OLD_PERIOD_END${NC}"
echo ""

# Step 3: Simulate Google Pub/Sub renewal webhook
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

# Step 4: Verify extension
echo -e "${YELLOW}[4/4] Verifying current_period_end was extended${NC}"

NEW_PERIOD_END=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SUB_QUERY" -t 2>/dev/null | tr -d '[:space:]')

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
