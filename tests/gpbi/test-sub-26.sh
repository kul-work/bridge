#!/bin/bash

##############################################################################
# SUB-26: Webhook Subscription Renewal Pending (Type 21)
# 
# Purpose: Verify that a subscriptionNotification with notificationType: 21
#          (RENEWAL_PENDING) is properly received and updates subscription status.
#
# Usage: ./test-sub-26.sh [--token "purchase_token"]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
PURCHASE_TOKEN=""
USER_ID="test_sub_user_26_$(date +%s)"

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
echo "SUB-26: Webhook Subscription Renewal Pending (Type 21)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure subscription record exists
echo -e "${YELLOW}[1/3] Verifying subscription record exists${NC}"

# Extract DB password if needed
if [[ -n "${BRIDGE_DB_URL:-}" ]]; then
    export PGPASSWORD="${BRIDGE_DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

if [[ -z "$PURCHASE_TOKEN" ]]; then
    # Always generate a unique token for each run to bypass deduplication
    PURCHASE_TOKEN="test-sub-setup-26-$(date +%s)-$RANDOM"
    echo -e "${YELLOW}creating setup record for token: $PURCHASE_TOKEN...${NC}"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, created_at, updated_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true, NOW() + INTERVAL '1 month', NOW(), NOW());" > /dev/null
    echo -e "${GREEN}✓ Created test subscription record${NC}"
fi

echo "Purchase Token: $PURCHASE_TOKEN"
echo ""

# Step 2: Send webhook
TIMESTAMP=$(date +%s000)
MESSAGE_ID="webhook-sub-pending-$(date +%s)-$RANDOM"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 21,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

echo -e "${YELLOW}[2/3] Sending SUBSCRIPTION_RENEWAL_PENDING webhook...${NC}"
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

echo -e "${GREEN}✓ Webhook sent${NC}"
sleep 2

# Step 3: Verify in DB
echo -e "${YELLOW}[3/3] Verifying status in DB...${NC}"
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t | tr -d '[:space:]')

if [[ "$STATUS" == "pending" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'pending'${NC}"
    exit 0
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected 'pending'${NC}"
    exit 1
fi
