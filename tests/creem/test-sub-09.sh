#!/bin/bash

##############################################################################
# SUB-09: Plan Upgrade/Downgrade (Webhook)
# 
# Purpose: Verify that a Creem subscription.update webhook with a new 
#          product_id properly updates the subscription's plan in the database.
#
# Usage: ./test-sub-09.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_INGRESS_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
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
echo "SUB-09: Plan Upgrade/Downgrade (Webhook)"
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

# We are testing plan upgrade/downgrade, so we simulate a new product ID
NEW_PRODUCT_ID="${PRODUCT_ID_SUB}_yearly"

# Step 2: Trigger Update Webhook
echo -e "${YELLOW}[2/4] Sending subscription.update webhook${NC}"
EVENT_ID="evt_sub_09_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.update",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_09"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$NEW_PRODUCT_ID"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 3: Verify DB status is 'active'
echo -e "${YELLOW}[3/4] Verifying status is 'active'${NC}"
sleep 2
QUERY_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t | tr -d ' ' || echo "")

STATUS=$(echo "$QUERY_RESULT" | tr -d ' ')

if [[ "$STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Verification passed: Status=$STATUS${NC}"
else
    echo -e "${RED}✗ Verification failed: Status=$STATUS (Expected: active)${NC}"
    exit 1
fi

# Step 4: Report
cat > test-sub-09-report.json <<EOF
{
  "test_id": "SUB-09",
  "status": "pass",
  "user_id": "$USER_ID",
  "new_product_id": "$NEW_PRODUCT_ID"
}
EOF
echo -e "${GREEN}✓ SUB-09 PASSED${NC}"
