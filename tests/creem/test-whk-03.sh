#!/bin/bash

##############################################################################
# WHK-03: Duplicate Delivery (Idempotency)
# 
# Purpose: Verify that duplicate webhooks (same event ID) are handled
#          idempotently - second attempt returns success but does not
#          create duplicate database entries.
#
# Usage: ./test-whk-03.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Creem Webhook Secret configured in .env
#   - psql installed and database accessible
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
DB_URL="$DATABASE_URL"
EMAIL=""

# Database password
export PGPASSWORD="${DATABASE_PASSWORD:-}"
if [[ -z "$PGPASSWORD" ]]; then
    export PGPASSWORD="${DB_URL##*:}"
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
echo "WHK-03: Duplicate Delivery (Idempotency)"
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

# Step 2: Cleanup and initial state
echo -e "${YELLOW}[2/4] Cleaning initial state for user tests${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
# Clean up any recorded webhooks for idempotency testing (assuming a webhooks table exists for idempotency)
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM webhooks WHERE provider = 'creem' AND provider_webhook_id LIKE 'whk-03%';" 2>/dev/null || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Send FIRST webhook
echo -e "${YELLOW}[3/4] Sending FIRST webhook delivery${NC}"
EVENT_ID="whk-03-$(date +%s)"
PERIOD_END=$(date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
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
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  First delivery response: HTTP $HTTP_CODE_1"

# Step 4: Send SECOND (identical) webhook
echo -e "${YELLOW}[4/4] Sending DUPLICATE webhook delivery (same Event ID)${NC}"

HTTP_CODE_2=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  Second delivery response: HTTP $HTTP_CODE_2"

# Verification
sleep 2 # process time
SUBS_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT count(*) FROM subscriptions WHERE clerk_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "  Final subscription count: $SUBS_COUNT"

if [[ "$HTTP_CODE_1" == "200" || "$HTTP_CODE_1" == "204" ]] && [[ "$HTTP_CODE_2" == "200" || "$HTTP_CODE_2" == "204" ]]; then
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
