#!/bin/bash

##############################################################################
# NET-02: Concurrent Deliveries (Race Condition)
# 
# Purpose: Verify that the backend handles near-simultaneous webhook
#          deliveries for the same event ID without creating duplicate
#          database records (Race Condition handling).
#
# Usage: ./test-net-02.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Creem Webhook Secret configured in .env
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/.env" ]; then
    # Load variables from .env
    set -a
    source "$SCRIPT_DIR/.env"
    set +a
fi
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Defaults
EMAIL=""

# Database password
export PGPASSWORD="${DATABASE_PASSWORD:-}"
if [[ -z "$PGPASSWORD" ]]; then
    export PGPASSWORD="${DATABASE_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "NET-02: Concurrent Deliveries (Race Condition)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Fetch user_id
echo -e "${YELLOW}[1/4] Fetching user_id for: $EMAIL${NC}"
USER_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT clerk_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ User not found or DB error${NC}"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"

# Step 2: Cleanup and prepare payload
echo -e "${YELLOW}[2/4] Preparing payload and cleaning state${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM webhooks WHERE provider = 'creem' AND provider_webhook_id LIKE 'net-02%';" 2>/dev/null || true

EVENT_ID="net-02-race-$(date +%s)"
PERIOD_END=$(date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "object": {
    "id": "$SUBSCRIPTION_ID",
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
echo -e "${YELLOW}[3/4] Triggering 5 concurrent webhook deliveries (Race Condition Test)${NC}"

for i in {1..5}; do
  curl -s -o /dev/null -w "Delivery $i: HTTP %{http_code}\n" -X POST \
    "$APP_URL/webhooks/creem" \
    -H "Content-Type: application/json" \
    -H "creem-signature: $SIGNATURE" \
    -d "$PAYLOAD" &
done

echo "Waiting for all background processes to complete..."
wait
echo -e "${GREEN}✓ All requests finished${NC}"

# Step 4: Verify DB
echo -e "${YELLOW}[4/4] Verifying database for duplicate records${NC}"
sleep 2 # process time

SUBS_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT count(*) FROM subscriptions WHERE clerk_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
WEBHOOK_LOG_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT count(*) FROM webhooks WHERE provider = 'creem' AND provider_webhook_id = '$EVENT_ID';" -t 2>/dev/null | tr -d ' ')

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
