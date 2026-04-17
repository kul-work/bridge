#!/bin/bash

##############################################################################
# SUB-PAUSE-02: Pause Takes Effect (Auto-Transition) Test
#
# Purpose: Verify that a subscription scheduled pause transitions to active 
#          pause when the scheduled date arrives (triggered by Type 10 webhook).
#          1. Establish a subscription with a PAST scheduled pause date
#          2. Send Type 10 webhook (subscription.paused)
#          3. Verify status changed to 'paused'
#          4. Verify google_paused_at is set in DB
#
# Usage: ./test-sub-pause-02.sh
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
DUMMY_TOKEN="test-sub-pause-02-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-pause-02-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-PAUSE-02: Pause Takes Effect"
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

# Step 3: Establish subscription with PAST scheduled pause date
echo -e "${YELLOW}[1/5] Seeding subscription with PAST scheduled pause date${NC}"

# Seed the database directly for this scenario
PAST_PAUSE_DATE=$(date -u -d '1 hour ago' '+%Y-%m-%d %H:%M:%S' 2>/dev/null || date -u -v-1H '+%Y-%m-%d %H:%M:%S')

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, google_pause_scheduled_at) 
      VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'active', true, '$DUMMY_TOKEN', '$PAST_PAUSE_DATE');" > /dev/null

echo -e "${GREEN}✓ Active subscription with scheduled pause seeded (Scheduled for: $PAST_PAUSE_DATE)${NC}"
echo ""

# Step 4: Send Type 10 webhook (paused)
echo -e "${YELLOW}[2/5] Sending subscription.paused webhook (notificationType 10)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 10,
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
      \"message_id\": \"test-webhook-pause-02-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Pause effective webhook sent${NC}"
echo ""

# Step 5: Verify status changed to 'paused' and google_paused_at is set
echo -e "${YELLOW}[3/5] Verifying pause state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, (google_paused_at IS NOT NULL) as is_paused_at_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: paused | t
if [[ "$RES_DATA" == *"paused"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Status is 'paused' and paused_at date is set${NC}"
else
    echo -e "${RED}✗ Failure: Pause state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-PAUSE-02",
  "test_name": "Pause Takes Effect (Auto-Transition)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "results": {
    "subscription_seeded": true,
    "pause_webhook_sent": true,
    "pause_effective": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-PAUSE-02 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0

