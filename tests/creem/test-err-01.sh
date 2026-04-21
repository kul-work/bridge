#!/bin/bash

##############################################################################
# ERR-01: Missing metadata.user_id
# 
# Purpose: Verify that webhooks missing metadata.user_id are handled
#          gracefully by the backend (logged but not causing crashes).
#
# Usage: ./test-err-01.sh
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
#     * PRODUCT_ID_SUB (for payload)
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

echo -e "${YELLOW}========================================${NC}"
echo "ERR-01: Missing metadata.user_id"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Prepare payload WITHOUT metadata.user_id
echo -e "${YELLOW}[1/3] Preparing payload missing metadata.user_id${NC}"
EVENT_ID="err-01-missing-user-$(date +%s)"
SUB_ID="sub_err_01_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUB_ID",
    "customer": {
      "email": "orphan@example.com",
      "id": "cust_err_01"
    },
    "metadata": {
      "some_other_key": "exists"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

# Step 2: Send Webhook
echo -e "${YELLOW}[2/3] Sending webhook (expecting acceptance or 400 but no crash)${NC}"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  Response: HTTP $HTTP_CODE"

# Step 3: Verify no record created
echo -e "${YELLOW}[3/3] Verifying no records created in pay.subscriptions${NC}"
sleep 2
SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE subscription_id = '$SUB_ID';" -t | tr -d '[:space:]' || echo "0")

if [[ "$SUBS_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ No orphaned subscription record created.${NC}"
    echo -e "\n${GREEN}✓ ERR-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Error handling logic failed: Record found!${NC}"
    exit 1
fi
