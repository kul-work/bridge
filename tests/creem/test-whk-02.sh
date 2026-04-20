#!/bin/bash

##############################################################################
# WHK-02: Invalid Signature Rejection
# 
# Purpose: Verify that webhooks with invalid signatures are rejected 
#          and do NOT modify the database.
#
# Usage: ./test-whk-02.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_TOKEN
#   - psql installed and database accessible
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
echo "WHK-02: Invalid Signature Rejection"
echo -e "${YELLOW}========================================${NC}"

# Step 2: Record initial state
echo -e "${YELLOW}[2/4] Recording initial database state for user $USER_ID${NC}"
INITIAL_SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]' || echo "0")
echo "  Initial Subscriptions: $INITIAL_SUBS_COUNT"

# Step 3: Send Webhook with INVALID signature
echo -e "${YELLOW}[3/4] Sending webhook with INVALID signature${NC}"
EVENT_ID="whk-02-invalid-$(date +%s)"
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
      "id": "cust_whk_02_invalid"
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

BAD_SIGNATURE="BAD_SIG_1234567890abcdef"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $BAD_SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "401" || "$HTTP_CODE" == "403" || "$HTTP_CODE" == "400" ]]; then
    echo -e "${GREEN}✓ Webhook correctly rejected with HTTP $HTTP_CODE${NC}"
else
    echo -e "${RED}✗ Webhook NOT rejected adequately (HTTP $HTTP_CODE)${NC}"
fi

# Step 4: Verify DB unchanged
echo -e "${YELLOW}[4/4] Verifying database remains unchanged${NC}"
sleep 2 # process time

FINAL_SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]' || echo "0")

echo "  Final Subscriptions: $FINAL_SUBS_COUNT"

if [[ "$FINAL_SUBS_COUNT" == "$INITIAL_SUBS_COUNT" ]]; then
    echo -e "${GREEN}✓ Database state unchanged as expected${NC}"
    echo -e "\n${GREEN}✓ WHK-02 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Database state MODIFIED despite invalid signature!${NC}"
    exit 1
fi
