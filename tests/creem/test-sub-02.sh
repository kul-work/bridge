#!/bin/bash

##############################################################################
# SUB-02: Subscription Renewal (Webhook)
# 
# Purpose: Verify that a Creem subscription.active (renewal) webhook properly 
#          extends the subscription period in the database.
#
# Usage: ./test-sub-02.sh [--email "user@example.com"] [--user-id "test_user"]
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

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="creem-sub-02-${TIMESTAMP}-$$"
REPORT_FILE="test-sub-02-report.json"
EMAIL="creem_user_${TEST_RUN_ID}@example.com"
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
    # Generate a unique USER_ID for this run
    USER_ID="creem_user_$TEST_RUN_ID"
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-02: Subscription Renewal (Webhook)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Ensure existing subscription exists
echo -e "${YELLOW}[1/4] Checking for existing active subscription${NC}"
SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

if [[ -z "$SUB_RESULT" ]]; then
    echo -e "${YELLOW}No active sub found. Running SUB-01 first...${NC}"
    ./test-sub-01.sh --user-id "$USER_ID"
    
    SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "SELECT current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")
fi

OLD_EXPIRY="$SUB_RESULT"
echo -e "${GREEN}✓ Current Expiry: $OLD_EXPIRY${NC}"

# Step 2: Trigger Renewal Webhook
echo -e "${YELLOW}[2/4] Sending renewal subscription.active webhook${NC}"
EVENT_ID="evt_sub_02_$TEST_RUN_ID"
# Set new expiry 60 days out (renewal)
NEW_EXPIRY=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+60 days" 2>/dev/null || date -u -v+60d +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_02"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$NEW_EXPIRY",
    "last_transaction": {
      "amount": 2999
    }
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

# Step 3: Verify DB update
echo -e "${YELLOW}[3/4] Verifying expiry date updated${NC}"
sleep 4
UPDATED_EXPIRY=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t | tr -d '[:space:]' || echo "")

if [[ "$UPDATED_EXPIRY" != "$OLD_EXPIRY" ]]; then
    echo -e "${GREEN}✓ Expiry updated to: $UPDATED_EXPIRY${NC}"
else
    echo -e "${RED}✗ Expiry date did not change!${NC}"
    exit 1
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-02",
  "test_name": "Subscription Renewal (Webhook)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "old_expiry": "$OLD_EXPIRY",
  "new_expiry": "$UPDATED_EXPIRY",
  "results": {
    "webhook_accepted": true,
    "expiry_updated": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-02 PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
exit 0
