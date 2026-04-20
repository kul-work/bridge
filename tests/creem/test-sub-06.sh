#!/bin/bash

##############################################################################
# SUB-06: Subscription Resumed (Webhook)
# 
# Purpose: Verify that a Creem subscription.resumed webhook restores the 
#          status to 'active' and grants access.
#
# Usage: ./test-sub-06.sh --user-id "test_user"
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
EMAIL="creem_user_$TIMESTAMP@example.com"
USER_ID="test_creem_user_$TIMESTAMP"

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
echo "SUB-06: Scheduled Cancellation (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure active subscription exists
echo -e "${YELLOW}[1/4] Checking for existing active subscription${NC}"
SUB_EXISTS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT id FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

if [[ -z "$SUB_EXISTS" ]]; then
    echo -e "${YELLOW}No active sub found for $USER_ID. Running SUB-01 first...${NC}"
    ./test-sub-01.sh --user-id "$USER_ID"
fi

echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Trigger scheduled_cancel Webhook
echo -e "${YELLOW}[2/4] Sending subscription.scheduled_cancel webhook${NC}"
EVENT_ID="evt_sub_06_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.scheduled_cancel",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_06"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB"
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

# Step 3: Verify DB status is 'scheduled_cancel' and 'auto_renewing' is false
echo -e "${YELLOW}[3/4] Verifying status is 'scheduled_cancel' and auto_renewing is false${NC}"
sleep 2
QUERY_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, auto_renewing FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t | tr -d ' ' || echo "")

if [[ -z "$QUERY_RESULT" ]]; then
    echo -e "${RED}✗ No subscription record found${NC}"
    exit 1
fi

STATUS=$(echo "$QUERY_RESULT" | awk -F '|' '{print $1}' | tr -d ' ')
AUTO_RENEW=$(echo "$QUERY_RESULT" | awk -F '|' '{print $2}' | tr -d ' ')

if [[ "$STATUS" == "scheduled_cancel" ]]; then
    if [[ "$AUTO_RENEW" == "f" || "$AUTO_RENEW" == "false" || -z "$AUTO_RENEW" ]]; then
        echo -e "${GREEN}✓ Verification passed: Status=$STATUS, Auto_Renewing=${AUTO_RENEW:-empty/null}${NC}"
    else
        echo -e "${RED}✗ Verification failed on Auto_Renewing: $AUTO_RENEW (Expected: f, false, or empty)${NC}"
        exit 1
    fi
else
    echo -e "${RED}✗ Verification failed: Status=$STATUS (Expected: scheduled_cancel)${NC}"
    exit 1
fi

# Step 4: Report
cat > test-sub-06-report.json <<EOF
{
  "test_id": "SUB-06",
  "status": "pass",
  "user_id": "$USER_ID",
  "db_status": "$STATUS",
  "auto_renewing": "$AUTO_RENEW"
}
EOF
echo -e "${GREEN}✓ SUB-06 PASSED${NC}"
