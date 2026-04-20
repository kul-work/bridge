#!/bin/bash

##############################################################################
# WHK-01: Valid Signature Acceptance
# 
# Purpose: Verify that legitimate webhooks with correct creem-signature
#          are accepted and processed by the backend.
#
# Usage: ./test-whk-01.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
#   - globals.cfg sourced
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Defaults
TIMESTAMP=$(date +%s)
EMAIL="creem_whk_user_$TIMESTAMP@example.com"
USER_ID="test_whk_user_$TIMESTAMP"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --user-id)
            USER_ID="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "WHK-01: Valid Signature Acceptance"
echo -e "${YELLOW}========================================${NC}"

# Step 2: Cleanup and setup state
echo -e "${YELLOW}[2/4] Cleaning up old data for user $USER_ID${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Trigger Webhook with VALID signature
echo -e "${YELLOW}[3/4] Sending subscription.active webhook with VALID signature${NC}"
EVENT_ID="whk-01-$(date +%s)"
PERIOD_END=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+30 days" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_whk_01"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$PERIOD_END"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 4: Verify DB
echo -e "${YELLOW}[4/4] Verifying database state${NC}"
sleep 2 # process time

SUBS_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

if [[ "$SUBS_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Subscription record created and status is active${NC}"
    echo -e "\n${GREEN}✓ WHK-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Database state inconsistent: Status=$SUBS_STATUS${NC}"
    exit 1
fi
