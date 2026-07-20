#!/bin/bash

##############################################################################
# ERR-07: Unknown Notification Type
# 
# Purpose: Verify that webhooks with unknown or future notification types 
#          (e.g., type 99) are acknowledged with HTTP 204 but no business action
#          is taken (forward-compatible).
#
# Usage: ./test-err-07.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: POST /webhooks/... returns HTTP 204 for unknown 'notificationType' values.
#                      The event and delivery are durably recorded but ignored by business logic.
#                      No state changes occur in pay.subscriptions or pay.payments.
#                      Ensures forward-compatibility with future provider updates.
#                      Validates that the system safely ignores unrecognized events.
#                      Confirms that the ingress 'switch' logic handles the default case gracefully.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="err-07-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="err-07-report.json"
USER_ID="${USER_ID:-test_err_07_user_$TEST_RUN_ID}"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
HTTP_99=0
HTTP_50=0
HTTP_100=0
HTTP_255=0
CURRENT_FAILURE_KIND="setup"
rm -f "$REPORT_FILE"

write_failure_report() {
    local failure_kind="$1"
    local failure_step="$2"
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-07",
  "test_name": "Unknown Notification Type",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "failure_kind": "$failure_kind",
  "failure_step": "$failure_step",
  "http_codes": [$HTTP_99, $HTTP_50, $HTTP_100, $HTTP_255]
}
EOF
}

fail_test() {
    write_failure_report "$1" "$2"
    exit 1
}

write_failure_report_on_exit() {
    local exit_code=$?
    if [[ "$exit_code" -ne 0 && ! -f "$REPORT_FILE" ]]; then
        write_failure_report "$CURRENT_FAILURE_KIND" "script_exit"
    fi
}
trap write_failure_report_on_exit EXIT

# Extract DB password once
# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-07: Unknown Notification Type"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/5] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    fail_test "setup" "user_id"
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Record initial database state
echo -e "${YELLOW}[2/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo -e "${BLUE}Initial payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo ""

# Step 3: Send webhooks with unknown notification types
CURRENT_FAILURE_KIND="behavior"
echo -e "${YELLOW}[3/5] Sending webhooks with unknown notification types${NC}"
echo ""

TIMESTAMP=$(date +%s000)
PURCHASE_TOKEN="test-err-07-unknown-type-$(date +%s)"

# Test with various unknown notification types
UNKNOWN_TYPES=(99 50 100 255)

ALL_ACKNOWLEDGED="true"

for TYPE in "${UNKNOWN_TYPES[@]}"; do
    echo -e "${BLUE}Testing notification type: $TYPE (unknown)${NC}"
    
    MESSAGE_ID="webhook-err-07-$TYPE-$TEST_RUN_ID"
    TIMESTAMP_MS=$(date +%s000)
    
    # Create notification with unknown type
    NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": $TYPE,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
    
    NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)
    
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
      "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-token" \
      -H "X-Webhook-Verification-Mode: off" \
      -d "{
        \"message\": {
          \"data\": \"$NOTIFICATION_B64\",
          \"message_id\": \"$MESSAGE_ID\",
          \"attributes\": {}
        },
        \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
      }")
    
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    BODY=$(echo "$RESPONSE" | sed '$d')

    case "$TYPE" in
        99) HTTP_99=$HTTP_CODE ;;
        50) HTTP_50=$HTTP_CODE ;;
        100) HTTP_100=$HTTP_CODE ;;
        255) HTTP_255=$HTTP_CODE ;;
    esac
    
    if [[ "$HTTP_CODE" == "204" ]] && [[ -z "$BODY" ]]; then
        echo -e "  ${GREEN}✓ Durably acknowledged with HTTP 204${NC}"
    else
        echo -e "  ${RED}✗ Expected an empty HTTP 204 response, got HTTP $HTTP_CODE${NC}"
        ALL_ACKNOWLEDGED="false"
    fi
done
echo ""

# Step 4: Verify no database state change
echo -e "${YELLOW}[4/5] Verifying no database state change (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

# Check for test token pay.subscriptions
UNKNOWN_TOKEN_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
UNKNOWN_TOKEN_PAYMENT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE provider_purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
UNKNOWN_WEBHOOK_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider = 'google_play' AND provider_webhook_id LIKE 'webhook-err-07-%-$TEST_RUN_ID' AND event_type = 'SUBSCRIPTION_UNKNOWN' AND purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
UNKNOWN_DELIVERY_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_delivery wd JOIN pay.webhook_provider wp ON wp.id = wd.webhook_provider_id WHERE wp.app_id = '$BRIDGE_APP_ID' AND wp.provider_webhook_id LIKE 'webhook-err-07-%-$TEST_RUN_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Subscriptions with test token: $UNKNOWN_TOKEN_SUB"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$FINAL_PAYMENT_COUNT" == "$INITIAL_PAYMENT_COUNT" ]] && [[ "$UNKNOWN_TOKEN_SUB" == "0" ]] && [[ "$UNKNOWN_TOKEN_PAYMENT" == "0" ]] && [[ "$UNKNOWN_WEBHOOK_COUNT" == "4" ]] && [[ "$UNKNOWN_DELIVERY_COUNT" == "4" ]]; then
    echo -e "${GREEN}✓ Business state unchanged and all four events were durably recorded${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database state changed with unknown notification types!${NC}"
fi
echo ""

# Step 5: Summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

echo "Unknown notification types tested: ${UNKNOWN_TYPES[*]}"
echo ""

# Determine overall test status
if [[ "$ALL_ACKNOWLEDGED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-07 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-07 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-07",
  "test_name": "Unknown Notification Type",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "failure_kind": $([[ "$TEST_STATUS" == "pass" ]] && echo "null" || echo '"behavior"'),
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "all_unknown_types_acknowledged": $ALL_ACKNOWLEDGED,
    "no_db_state_change": $DB_UNCHANGED,
    "unknown_types_tested": [99, 50, 100, 255],
    "http_codes": [$HTTP_99, $HTTP_50, $HTTP_100, $HTTP_255],
    "unknown_token_subscription_count": $UNKNOWN_TOKEN_SUB,
    "unknown_token_payment_count": $UNKNOWN_TOKEN_PAYMENT,
    "unknown_webhook_count": $UNKNOWN_WEBHOOK_COUNT,
    "unknown_delivery_count": $UNKNOWN_DELIVERY_COUNT,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT
  },
  "notes": "Google may add new notification types; backend should be forward-compatible"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
