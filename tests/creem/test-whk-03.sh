#!/bin/bash

##############################################################################
# WHK-03: Duplicate Delivery (Idempotency)
# 
# Purpose: Verify that duplicate webhooks (same event ID) are handled
#          idempotently - second attempt returns success but does not
#          create duplicate database entries.
#
# Usage: ./test-whk-03.sh --user-id "test_user"
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
echo "WHK-03: Duplicate Delivery (Idempotency)"
echo -e "${YELLOW}========================================${NC}"

# Step 2: Cleanup and initial state
echo -e "${YELLOW}[2/4] Cleaning initial state for user $USER_ID${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
# Clean up recorded webhooks in pay.webhook_log
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.webhook_log WHERE provider = 'creem' AND provider_webhook_id LIKE 'whk-03%';" > /dev/null 2>&1 || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Send FIRST webhook
echo -e "${YELLOW}[3/4] Sending FIRST webhook delivery${NC}"
EVENT_ID="whk-03-$(date +%s)"
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
      "id": "cust_whk_03"
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

HTTP_CODE_1=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  First delivery response: HTTP $HTTP_CODE_1"

# Step 4: Send SECOND (identical) webhook
echo -e "${YELLOW}[4/4] Sending DUPLICATE webhook delivery (same Event ID)${NC}"

HTTP_CODE_2=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  Second delivery response: HTTP $HTTP_CODE_2"

# Verification
sleep 2 # process time
SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]' || echo "0")

echo "  Final subscription count: $SUBS_COUNT"

if [[ "$HTTP_CODE_1" =~ ^20[014]$ ]] && [[ "$HTTP_CODE_2" =~ ^20[014]$ ]]; then
    echo -e "  ${GREEN}✓ Both deliveries returned success${NC}"
    if [[ "$SUBS_COUNT" == "1" ]]; then
        echo -e "  ${GREEN}✓ No duplicate subscription record created (Idempotency PASSED)${NC}"
        echo -e "\n${GREEN}✓ WHK-03 PASSED${NC}"
        exit 0
    else
        echo -e "  ${RED}✗ Duplicate records created in DB (Idempotency FAILED)${NC}"
        exit 1
    fi
else
    echo -e "  ${RED}✗ Webhook processing failed (HTTP codes: $HTTP_CODE_1, $HTTP_CODE_2)${NC}"
    exit 1
fi
