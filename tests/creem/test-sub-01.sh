#!/bin/bash

##############################################################################
# SUB-01: New Subscription (Webhook)
# 
# Purpose: Verify that a successful Creem subscription.active webhook is 
#          properly verified and stored in the database with status 'active'.
#
# Usage: ./test-sub-01.sh --user-id "test_user"
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
EMAIL="creem_user_$TIMESTAMP@example.com"
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
echo "SUB-01: Initial Subscription (Active)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: User Identity
echo -e "${YELLOW}[1/5] Using External User ID: $USER_ID (Email: $EMAIL)${NC}"
echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Cleanup
echo -e "${YELLOW}[2/5] Cleaning up old data from Bridge DB${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' OR subscription_id = '$SUBSCRIPTION_ID';" > /dev/null 2>&1 || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' OR subscription_id = '$SUBSCRIPTION_ID';" > /dev/null 2>&1 || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.webhook_provider WHERE provider = 'creem' AND provider_webhook_id LIKE 'evt_sub_01_%';" > /dev/null 2>&1 || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Trigger Webhook
echo -e "${YELLOW}[3/5] Sending subscription.active webhook to Bridge${NC}"
EVENT_ID="evt_sub_01_$(date +%s)"

# Creem uses ISO 8601 for dates
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
      "id": "cust_creem_01"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$PERIOD_END",
    "last_transaction": {
      "amount": 2999
    }
  }
}
EOF
)

# Use HMAC-SHA256 with Creem Webhook Secret
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
echo -e "${YELLOW}[4/5] Verifying pay.subscriptions table${NC}"
sleep 2 # Allow async processing
QUERY="SELECT status, auto_renewing FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' LIMIT 1;"

SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "$QUERY" -t 2>/dev/null || echo "")

if [[ -z "$SUB_RESULT" ]] || [[ "$SUB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No subscription record found for query: $QUERY${NC}"
    exit 1
fi

STATUS=$(echo "$SUB_RESULT" | awk -F '|' '{print $1}' | tr -d '[:space:]')

if [[ "$STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Subscription verified: Status=$STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected Status: Status=$STATUS (expected 'active')${NC}"
    exit 1
fi

# Step 5: Report
cat > test-sub-01-report.json <<EOF
{
  "test_id": "SUB-01",
  "status": "pass",
  "user_id": "$USER_ID",
  "subscription_id": "$SUBSCRIPTION_ID",
  "event_id": "$EVENT_ID",
  "http_code": $HTTP_CODE,
  "db_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ SUB-01 PASSED${NC}"

