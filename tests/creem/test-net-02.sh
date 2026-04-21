#!/bin/bash

##############################################################################
# NET-02: Concurrent Deliveries (Race Condition)
# 
# Purpose: Verify that the backend handles near-simultaneous webhook
#          deliveries for the same event ID without creating duplicate
#          database records (Race Condition handling).
#
# Usage: ./test-net-02.sh [--email "user@example.com"] [--user-id "test_user"]
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

# Defaults
TIMESTAMP=$(date +%s)
EMAIL="creem_net_user_$TIMESTAMP@example.com"
USER_ID=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
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

if [[ -z "$USER_ID" ]]; then
    # Generate a stable-ish USER_ID from email if not provided
    USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"
fi

echo -e "${YELLOW}========================================${NC}"
echo "NET-02: Concurrent Deliveries (Race Condition)"
echo -e "${YELLOW}========================================${NC}"

# Step 2: Cleanup and prepare payload
echo -e "${YELLOW}[2/4] Preparing payload and cleaning state for user $USER_ID${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.webhook_provider WHERE provider = 'creem' AND provider_webhook_id LIKE 'net-02%';" > /dev/null 2>&1 || true

EVENT_ID="net-02-race-$(date +%s)"
PERIOD_END=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+30 days" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "sub-$EVENT_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_net_02_race"
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
echo -e "${GREEN}✓ Payload ready${NC}"

# Step 3: Trigger CONCURRENT webhooks
echo -e "${YELLOW}[3/4] Triggering 3 concurrent webhook deliveries (Race Condition Test)${NC}"

# We use 3 instead of 5 for safety in test environments
for i in {1..3}; do
  curl -s -o /dev/null -w "Delivery $i: HTTP %{http_code}\n" -X POST \
    "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
    -H "Content-Type: application/json" \
    -H "creem-signature: $SIGNATURE" \
    -d "$PAYLOAD" &
done

if [[ -z "$USER_ID" ]]; then
    # Generate a stable-ish USER_ID from email if not provided
    USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"
fi

echo "Waiting for all background processes to complete..."
wait
echo -e "${GREEN}✓ All requests finished${NC}"

# Step 4: Verify DB
echo -e "${YELLOW}[4/4] Verifying database for duplicate records${NC}"
sleep 2 # process time

SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]' || echo "0")
WEBHOOK_LOG_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.webhook_provider WHERE provider = 'creem' AND provider_webhook_id = '$EVENT_ID';" -t | tr -d '[:space:]' || echo "0")

echo "  Subscription records created: $SUBS_COUNT"
echo "  Webhook log entries: $WEBHOOK_LOG_COUNT"

if [[ "$SUBS_COUNT" == "1" && "$WEBHOOK_LOG_COUNT" == "1" ]]; then
    echo -e "\n${GREEN}✓ Race condition handled correctly. No duplicates created.${NC}"
    echo -e "${GREEN}✓ NET-02 PASSED${NC}"
    exit 0
else
    echo -e "\n${RED}✗ DUPLICATION DETECTED! Race condition handling FAILED.${NC}"
    echo "  Expected 1 record, found $SUBS_COUNT subscriptions and $WEBHOOK_LOG_COUNT webhook logs."
    exit 1
fi
