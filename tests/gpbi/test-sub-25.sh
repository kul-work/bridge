#!/bin/bash

##############################################################################
# SUB-25: Webhook Subscription Deferred (Type 9)
# 
# Purpose: Verify that a subscriptionNotification with notificationType: 9
#          (DEFERRED) is properly received and updates the deferred time.
#
# Usage: ./test-sub-25.sh [--token "purchase_token"]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars
#   - jq installed and in PATH
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for SUB-25 snapshot assertions"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_RUN_ID="sub-25-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
PURCHASE_TOKEN=""
USER_ID="test_sub_user_25_$TEST_RUN_ID"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --token)
            PURCHASE_TOKEN="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "SUB-25: Webhook Subscription Deferred (Type 9)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Ensure subscription record exists
echo -e "${YELLOW}[1/4] Verifying subscription record exists${NC}"

# Extract DB password if needed
if [[ -n "${BRIDGE_DB_URL:-}" ]]; then
    export PGPASSWORD="${BRIDGE_DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

if [[ -z "$PURCHASE_TOKEN" ]]; then
    # Always generate a unique user/token for each run to bypass deduplication
    USER_ID="test_sub_user_25_$TEST_RUN_ID"
    PURCHASE_TOKEN="test-sub-25-$TEST_RUN_ID"
    echo -e "${YELLOW}creating setup record for user $USER_ID, token: $PURCHASE_TOKEN...${NC}"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, created_at, updated_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true, NOW() + INTERVAL '1 month', NOW(), NOW());" > /dev/null
    echo -e "${GREEN}PASS: Created test subscription record${NC}"
else
    USER_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT external_user_id FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN' LIMIT 1;" -t | tr -d '[:space:]')

    if [[ -z "$USER_ID" ]]; then
        echo -e "${RED}FAIL: No subscription record found for token $PURCHASE_TOKEN${NC}"
        exit 1
    fi
fi

echo "User ID: $USER_ID"
echo "Purchase Token: $PURCHASE_TOKEN"
echo ""

# Step 2: Send webhook
TIMESTAMP_MS=$(date +%s000)
MESSAGE_ID="webhook-sub-25-$TEST_RUN_ID"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 9,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID",
    "deferredExpiryTimeMillis": 1900000000000
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

echo -e "${YELLOW}[2/4] Sending SUBSCRIPTION_DEFERRED webhook...${NC}"
curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}PASS: Webhook sent${NC}"
sleep 2

# Step 3: Verify in DB
echo -e "${YELLOW}[3/4] Verifying google_deferred_until in DB...${NC}"
DEFERRED_UNTIL=""
for attempt in $(seq 1 10); do
    DEFERRED_UNTIL=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT google_deferred_until FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t | tr -d '[:space:]')
    if [[ -n "$DEFERRED_UNTIL" ]]; then
        break
    fi
    sleep 1
done

if [[ -n "$DEFERRED_UNTIL" && "$DEFERRED_UNTIL" != "" ]]; then
    echo -e "${GREEN}PASS: google_deferred_until is set to $DEFERRED_UNTIL${NC}"
else
    echo -e "${RED}FAIL: google_deferred_until is not set${NC}"
    exit 1
fi

# Step 4: Verify app-facing subscription snapshot
echo -e "${YELLOW}[4/4] Verifying active deferred snapshot contract${NC}"
STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
STATUS_BODY=$(echo "$STATUS_RESPONSE" | sed '$d')

if [[ "$STATUS_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}FAIL: subscription-status returned HTTP $STATUS_HTTP_CODE${NC}"
    echo "$STATUS_BODY"
    exit 1
fi

if echo "$STATUS_BODY" | jq -e \
  '.is_premium == true and .status == "active" and .google_deferred_until != null' > /dev/null; then
    echo -e "${GREEN}PASS: Snapshot shows active access with google_deferred_until${NC}"
else
    echo -e "${RED}FAIL: Deferred snapshot contract mismatch${NC}"
    echo "$STATUS_BODY" | jq .
    exit 1
fi

exit 0
