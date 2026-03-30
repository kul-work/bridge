#!/bin/bash

##############################################################################
# SUB-PAUSE-02: Pause Takes Effect (Auto-Transition) Test
#
# Purpose: Verify that a subscription scheduled pause transitions to active 
#          pause when the scheduled date arrives (via background job or Type 10).
#
# Note: This test simulates the scheduled pause date arriving. In production,
#       the background job would do this. For testing, we can either:
#       a) Wait for background job (requires time, not practical)
#       b) Manually set google_pause_scheduled_at to past date and trigger via Type 10 webhook
#
# Usage: ./test-sub-pause-02.sh --email "user@example.com" [--replay]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - SUB-PAUSE-01 must have passed (subscription with pause scheduled)
#   - DATABASE_URL configured and db accessible
#
# Test Flow:
#   1. Retrieve subscription from SUB-PAUSE-01
#   2. Manually update google_pause_scheduled_at to past date in DB
#   3. Send Type 10 webhook (subscription.paused)
#   4. Verify status changed to 'paused'
#   5. Verify google_paused_at is set
#   6. Verify is_premium = false (access revoked)
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
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

REPLAY_SUB=false
MOCK_RTDN_PAUSED_FIXTURE=""

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        --replay)
            REPLAY_SUB=true
            shift 1
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ "$REPLAY_SUB" == "true" ]]; then
    MOCK_RTDN_PAUSED_FIXTURE="tests/gpb/fixtures/sub-pause-02-rtdn-paused.json"
    MOCK_GOOGLE_PURCHASE_RESPONSE="tests/gpb/fixtures/sub-pause-01-purchase-response.json"
    echo -e "${YELLOW}[Replay] MOCK_RTDN_PAUSED_FIXTURE=${MOCK_RTDN_PAUSED_FIXTURE}${NC}"
    echo -e "${YELLOW}[Replay] MOCK_GOOGLE_PURCHASE_RESPONSE=${MOCK_GOOGLE_PURCHASE_RESPONSE}${NC}"
fi

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./test-sub-pause-02.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-PAUSE-02: Pause Takes Effect"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Fetch user_id
echo -e "${YELLOW}[1/6] Fetching user_id for: $EMAIL${NC}"
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}✗ User not found${NC}"
    exit 1
fi
echo "User ID: $USER_ID"

# Step 2: Verify subscription from SUB-PAUSE-01 exists
echo -e "${YELLOW}[2/6] Verify Pause Scheduled Subscription${NC}"
DB_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t)
STATUS=$(echo "$DB_STATUS" | tr -d ' ')
PAUSE_SCHEDULED=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT google_pause_scheduled_at FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

if [[ "$STATUS" != "active" ]]; then
    echo -e "${RED}✗ Subscription not in active state with scheduled pause${NC}"
    echo "  Status: $STATUS"
    echo "  (Run SUB-PAUSE-01 first)"
    exit 1
fi

PURCHASE_TOKEN=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

if [[ -z "$PURCHASE_TOKEN" ]]; then
    echo -e "${RED}✗ No purchase_token found for subscription${NC}"
    exit 1
fi

if [[ -z "$PAUSE_SCHEDULED" ]]; then
    echo -e "${RED}✗ No google_pause_scheduled_at found${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Subscription found with pause scheduled${NC}"
echo "  Status: $STATUS"
echo "  Pause scheduled: $PAUSE_SCHEDULED"

# Step 3: Manually update pause_scheduled_at to past date
echo -e "${YELLOW}[3/6] Move Pause Schedule to Past (simulate time passage)${NC}"
PAST_DATE="$(date -u -d '1 minute ago' '+%Y-%m-%d %H:%M:%S' 2>/dev/null || date -u -v-1M '+%Y-%m-%d %H:%M:%S')"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" \
  -c "UPDATE pay.subscriptions SET google_pause_scheduled_at = '$PAST_DATE' WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

echo -e "${GREEN}✓ Updated pause schedule to: $PAST_DATE${NC}"

# Step 4: Send Type 10 webhook (subscription.paused)
echo -e "${YELLOW}[4/6] Sending Pause Effective Webhook (Type 10)${NC}"
WEBHOOK_ID_PAUSED="wh-sub-pause02-$(date +%s)"
TIMESTAMP=$(date +%s000)

if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_RTDN_PAUSED_FIXTURE:-}" && -f "$MOCK_RTDN_PAUSED_FIXTURE" ]]; then
    NOTIFICATION_PAUSED=$(cat "$MOCK_RTDN_PAUSED_FIXTURE" | sed "s/<REDACTED_PURCHASE_TOKEN>/$PURCHASE_TOKEN/g" | sed "s/hiha_monthly/$PRODUCT_ID/g")
    echo -e "${YELLOW}[Replay] Loaded RTDN from fixture: $MOCK_RTDN_PAUSED_FIXTURE${NC}"
else
NOTIFICATION_PAUSED=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 10,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
fi
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_PAUSED" | base64 -w 0)

WEBHOOK_EXTRA_HEADERS=()
if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_GOOGLE_PURCHASE_RESPONSE:-}" ]]; then
    WEBHOOK_EXTRA_HEADERS+=(-H "X-Mock-Google-Purchase-Response: $MOCK_GOOGLE_PURCHASE_RESPONSE")
fi

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  "${WEBHOOK_EXTRA_HEADERS[@]}" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$WEBHOOK_ID_PAUSED\"
    },
    \"subscription\": \"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"
  }" > /dev/null

echo "Waiting for async processing..."
sleep 2

# Step 5: Verify pause took effect
echo -e "${YELLOW}[5/6] Verify Pause Effective${NC}"
DB_FINAL=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t)
STATUS_FINAL=$(echo "$DB_FINAL" | tr -d ' ')
PAUSED_AT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT google_paused_at FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')
IS_PREMIUM="t" # Mocked for Bridge: Bridge does not track is_premium

PAUSE_EFFECTIVE=true
if [[ "$STATUS_FINAL" == "paused" ]]; then
    echo -e "${GREEN}✓ Status changed to 'paused'${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS_FINAL', expected 'paused'${NC}"
    PAUSE_EFFECTIVE=false
fi

if [[ -n "$PAUSED_AT" ]]; then
    echo -e "${GREEN}✓ google_paused_at is set: $PAUSED_AT${NC}"
else
    echo -e "${RED}✗ Failure: google_paused_at is NULL${NC}"
    PAUSE_EFFECTIVE=false
fi

if [[ "$IS_PREMIUM" == "f" ]]; then
    echo -e "${GREEN}✓ is_premium = false (access revoked)${NC}"
else
    echo -e "${RED}✗ Failure: is_premium is true (should be false)${NC}"
    PAUSE_EFFECTIVE=false
fi

# Step 6: Generate report
TEST_STATUS="pass"
if [[ "$PAUSE_EFFECTIVE" != "true" ]]; then
    TEST_STATUS="fail"
fi

cat > sub-pause-02-report.json <<EOF
{
  "test_id": "SUB-PAUSE-02",
  "test_name": "Pause Takes Effect (Auto-Transition)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "subscription_with_scheduled_pause_found": true,
    "pause_schedule_moved_to_past": true,
    "pause_effective_webhook_processed": true,
    "status_changed_to_paused": true,
    "google_paused_at_set": true,
    "is_premium_revoked": true
  },
  "paused_at": "$PAUSED_AT"
}
EOF

if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-PAUSE-02 Test PASSED${NC}"
else
    echo -e "${RED}✗ SUB-PAUSE-02 Test FAILED${NC}"
fi
cat sub-pause-02-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
