#!/bin/bash

##############################################################################
# SUB-PAUSE-03: Manual Resume from Pause Test
#
# Purpose: Verify that a paused subscription can be manually resumed by the 
#          user (triggered by Type 1 webhook).
#          1. Establish a subscription in 'paused' state
#          2. Send Type 1 webhook (subscription.recovered)
#          3. Verify status changed back to 'active'
#          4. Verify google_paused_at is cleared in DB
#
# Usage: ./test-sub-pause-03.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * PRODUCT_ID_SUB, PROVIDER, PACKAGE_NAME
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
DUMMY_TOKEN="test-sub-pause-03-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-pause-03-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-PAUSE-03: Manual Resume from Pause"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo ""

# Step 3: Establish subscription in 'paused' state
echo -e "${YELLOW}[1/5] Seeding subscription in 'paused' state${NC}"

# Seed the database directly for this scenario
PAST_PAUSED_AT=$(date -u -d '1 day ago' '+%Y-%m-%d %H:%M:%S' 2>/dev/null || date -u -v-1d '+%Y-%m-%d %H:%M:%S')

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, google_paused_at) 
      VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'paused', true, '$DUMMY_TOKEN', '$PAST_PAUSED_AT');" > /dev/null

echo -e "${GREEN}✓ Paused subscription seeded in DB (Paused at: $PAST_PAUSED_AT)${NC}"
echo ""

# Step 4: Send Type 7 webhook (restarted - resume from pause)
echo -e "${YELLOW}[2/5] Sending subscription.restarted webhook (notificationType 7)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 7,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"test-webhook-pause-03-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Recovery webhook sent${NC}"
echo ""

# Step 5: Verify status changed back to 'active' and google_paused_at is cleared
echo -e "${YELLOW}[3/5] Verifying resumed state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, (google_paused_at IS NULL) as is_paused_at_cleared FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: active | t
if [[ "$RES_DATA" == *"active"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'active' and paused_at date is cleared${NC}"
else
    echo -e "${RED}✗ Failure: Resumed state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-PAUSE-03",
  "test_name": "Manual Resume from Pause",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "results": {
    "subscription_seeded": true,
    "resume_webhook_sent": true,
    "resume_effective": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-PAUSE-03 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
