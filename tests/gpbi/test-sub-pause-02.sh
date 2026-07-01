#!/bin/bash

##############################################################################
# SUB-PAUSE-02: Pause Takes Effect (Auto-Transition)
#
# Purpose: Verify subscription transitions to 'paused' on schedule.
#
# Usage: ./test-sub-pause-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#   - jq installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: subscription.paused (10) is processed.
#                      Subscription status transitions to 'paused'.
#                      'google_paused_at' timestamp is populated.
#                      Ensures scheduled pauses correctly transition to enforcement.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for SUB-PAUSE-02 snapshot assertions"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="sub-pause-02-${TIMESTAMP}-$$"
DUMMY_TOKEN="test-sub-pause-02-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-pause-02-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-PAUSE-02: Pause Takes Effect"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}PASS: Testing with User ID: $USER_ID${NC}"
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

echo -e "${GREEN}PASS: Active subscription with scheduled pause seeded (Scheduled for: $PAST_PAUSE_DATE)${NC}"
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
      \"message_id\": \"test-webhook-pause-02-$TEST_RUN_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}PASS: Pause effective webhook sent${NC}"
echo ""

# Step 5: Verify status changed to 'paused' and google_paused_at is set
echo -e "${YELLOW}[3/5] Verifying pause state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=""
for attempt in $(seq 1 10); do
    RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT status, (google_paused_at IS NOT NULL) as is_paused_at_set FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')
    if [[ "$RES_DATA" == *"paused"*"t"* ]]; then
        break
    fi
    sleep 1
done

# Expected: paused | t
if [[ "$RES_DATA" == *"paused"*"t"* ]]; then
    echo -e "${GREEN}PASS: Status is 'paused' and paused_at date is set${NC}"
else
    echo -e "${RED}FAIL: Pause state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Step 6: Verify app-facing subscription snapshot
echo -e "${YELLOW}[4/5] Verifying paused snapshot contract${NC}"
STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
STATUS_BODY=$(echo "$STATUS_RESPONSE" | sed '$d')

if [[ "$STATUS_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}FAIL: subscription-status returned HTTP $STATUS_HTTP_CODE${NC}"
    echo "$STATUS_BODY"
    exit 1
fi

if echo "$STATUS_BODY" | jq -e \
  '.is_premium == false and .status == "paused"' > /dev/null; then
    echo -e "${GREEN}PASS: Snapshot shows paused access revoked${NC}"
else
    echo -e "${RED}FAIL: Paused snapshot contract mismatch${NC}"
    echo "$STATUS_BODY" | jq .
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-PAUSE-02",
  "test_name": "Pause Takes Effect (Auto-Transition)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "results": {
    "subscription_seeded": true,
    "pause_webhook_sent": true,
    "pause_effective": true,
    "snapshot_verified": true
  }
}
EOF

echo -e "${GREEN}PASS: SUB-PAUSE-02 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
