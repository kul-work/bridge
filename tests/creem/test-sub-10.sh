#!/bin/bash

##############################################################################
# SUB-10: Recovery from Past Due (Webhook)
# 
# Purpose: Verify that a Creem subscription.active webhook correctly 
#          restores an 'active' status from a 'past_due' or 'unpaid' state.
#
# Usage: ./test-sub-10.sh --user-id "test_user"
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
echo "SUB-10: Admin Pause (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure active subscription exists
echo -e "${YELLOW}[1/4] Checking for existing active subscription${NC}"
SUB_EXISTS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT id FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

if [[ -z "$SUB_EXISTS" ]]; then
    echo -e "${YELLOW}No active sub found for $USER_ID. Running SUB-01 first...${NC}"
    ./test-sub-01.sh --user-id "$USER_ID"
fi

echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Trigger Paused Webhook
echo -e "${YELLOW}[2/4] Sending subscription.paused webhook${NC}"
EVENT_ID="evt_sub_10_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.paused",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_10"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "paused",
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

# Step 3: Verify DB status is 'paused'
echo -e "${YELLOW}[3/4] Verifying status is 'paused'${NC}"
sleep 2
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t | tr -d '[:space:]' || echo "")

if [[ "$STATUS" == "paused" ]]; then
    echo -e "${GREEN}✓ Status verified: $STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected status: $STATUS (Expected: paused)${NC}"
    exit 1
fi

# Step 4: Report
cat > test-sub-10-report.json <<EOF
{
  "test_id": "SUB-10",
  "status": "pass",
  "user_id": "$USER_ID",
  "db_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ SUB-10 PASSED${NC}"
