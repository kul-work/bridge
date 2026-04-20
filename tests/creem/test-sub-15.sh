#!/bin/bash

##############################################################################
# SUB-15: Admin Resumes Paused Subscription (Webhook)
# 
# Purpose: Verify that a Creem subscription.resumed webhook (triggered by 
#          admin) restores the subscription status to 'active' and grants access.
#
# Usage: ./test-sub-15.sh --user-id "test_user"
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
echo "SUB-15: Admin Resumes Paused Subscription"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure subscription exists in paused state
echo -e "${YELLOW}[1/4] Checking for existing paused subscription${NC}"
STATUS_CHECK=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t | tr -d ' ' || echo "")
STATUS_VAL=$(echo "$STATUS_CHECK" | tr -d ' ')

if [[ "$STATUS_VAL" != "paused" ]]; then
    echo -e "${YELLOW}Subscription not in paused state. Running SUB-10 first...${NC}"
    ./test-sub-10.sh --user-id "$USER_ID"
    
    STATUS_CHECK=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t | tr -d ' ' || echo "")
    STATUS_VAL=$(echo "$STATUS_CHECK" | tr -d ' ')
fi

echo -e "${GREEN}✓ Ready (Status: $STATUS_VAL)${NC}"

# Step 2: Trigger Active Webhook (subscription.active for resume)
echo -e "${YELLOW}[2/4] Sending subscription.active webhook to resume${NC}"
EVENT_ID="evt_sub_15_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_15"
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

# Step 3: Verify DB status is 'active'
echo -e "${YELLOW}[3/4] Verifying status returned to 'active'${NC}"
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
cat > test-sub-15-report.json <<EOF
{
  "test_id": "SUB-15",
  "status": "pass",
  "user_id": "$USER_ID",
  "db_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ SUB-15 PASSED${NC}"
