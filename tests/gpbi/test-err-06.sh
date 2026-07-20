#!/bin/bash

##############################################################################
# ERR-06: Webhook Payload Malformed
# 
# Purpose: Verify that malformed webhook payloads (invalid JSON, missing fields, 
#          bad base64) are rejected with HTTP 400 and no database state change.
#
# Usage: ./test-err-06.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PACKAGE_NAME, BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: malformed JSON/data returns HTTP 400 webhook_error.
#                      Decoding failures for base64 fields are correctly caught and rejected.
#                      A valid envelope without a known notification is durably recorded and returns HTTP 204.
#                      No subscription or payment state changes occur.
#                      Ensures ingress is resilient against data corruption and malformed input.
#                      Validates that the ingress validation layer identifies schema violations.
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
TEST_RUN_ID="err-06-${TIMESTAMP}-$$"
REPORT_FILE="err-06-report.json"
USER_ID="${USER_ID:-test_err_06_user_$TEST_RUN_ID}"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
HTTP_1=0
HTTP_2=0
HTTP_3=0
HTTP_4=0
HTTP_5=0
CURRENT_FAILURE_KIND="setup"
rm -f "$REPORT_FILE"

write_failure_report() {
    local failure_kind="$1"
    local failure_step="$2"
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-06",
  "test_name": "Webhook Payload Malformed",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "failure_kind": "$failure_kind",
  "failure_step": "$failure_step",
  "http_codes": [$HTTP_1, $HTTP_2, $HTTP_3, $HTTP_4, $HTTP_5]
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
echo "ERR-06: Webhook Payload Malformed"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/5] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    fail_test "setup" "user_id"
fi

if ! command -v jq >/dev/null 2>&1; then
    fail_test "setup" "jq"
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Record initial database state
echo -e "${YELLOW}[2/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo ""

# Step 3: Test with various malformed payloads
CURRENT_FAILURE_KIND="behavior"
echo -e "${YELLOW}[3/5] Testing with malformed webhook payloads${NC}"
echo ""

ALL_REJECTED="true"

# Test 1: Invalid JSON
echo -e "${BLUE}Test 1: Invalid JSON syntax${NC}"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{ invalid json }")

HTTP_1=$(echo "$RESPONSE_1" | tail -n1)
BODY_1=$(echo "$RESPONSE_1" | sed '$d')
if [[ "$HTTP_1" == "400" ]] && echo "$BODY_1" | jq -e '.error == "webhook_error" and (.message | startswith("Invalid JSON payload:"))' >/dev/null; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400 webhook_error${NC}"
else
    echo -e "  ${RED}✗ Expected HTTP 400 webhook_error for invalid JSON${NC}"
    ALL_REJECTED="false"
fi

# Test 2: Empty payload
echo -e "${BLUE}Test 2: Empty payload${NC}"
RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "")

HTTP_2=$(echo "$RESPONSE_2" | tail -n1)
BODY_2=$(echo "$RESPONSE_2" | sed '$d')
if [[ "$HTTP_2" == "400" ]] && echo "$BODY_2" | jq -e '.error == "webhook_error" and (.message | startswith("Invalid JSON payload:"))' >/dev/null; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400 webhook_error${NC}"
else
    echo -e "  ${RED}✗ Expected HTTP 400 webhook_error for empty payload${NC}"
    ALL_REJECTED="false"
fi

# Test 3: Missing message.data
echo -e "${BLUE}Test 3: Missing message.data field${NC}"
RESPONSE_3=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d '{
    "message": {
      "message_id": "err-06-missing-data"
    },
    "subscription": "projects/test/pay.subscriptions/test"
  }')

HTTP_3=$(echo "$RESPONSE_3" | tail -n1)
BODY_3=$(echo "$RESPONSE_3" | sed '$d')
if [[ "$HTTP_3" == "400" ]] && echo "$BODY_3" | jq -e '.error == "webhook_error" and .message == "Missing message.data field"' >/dev/null; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400 webhook_error${NC}"
else
    echo -e "  ${RED}✗ Expected HTTP 400 webhook_error for missing message.data${NC}"
    ALL_REJECTED="false"
fi

# Test 4: Invalid base64 in message.data
echo -e "${BLUE}Test 4: Invalid base64 encoding${NC}"
RESPONSE_4=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d '{
    "message": {
      "data": "not-valid-base64!!!",
      "message_id": "err-06-invalid-base64"
    },
    "subscription": "projects/test/pay.subscriptions/test"
  }')

HTTP_4=$(echo "$RESPONSE_4" | tail -n1)
BODY_4=$(echo "$RESPONSE_4" | sed '$d')
if [[ "$HTTP_4" == "400" ]] && echo "$BODY_4" | jq -e '.error == "webhook_error" and (.message | startswith("Invalid message.data:"))' >/dev/null; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400 webhook_error${NC}"
else
    echo -e "  ${RED}✗ Expected HTTP 400 webhook_error for invalid base64${NC}"
    ALL_REJECTED="false"
fi

# Test 5: Missing subscriptionNotification inside data
echo -e "${BLUE}Test 5: Missing subscriptionNotification in decoded data${NC}"
INCOMPLETE_JSON="{\"version\":\"1.0\",\"packageName\":\"$PACKAGE_NAME\"}"
INCOMPLETE_B64=$(echo -n "$INCOMPLETE_JSON" | base64 -w 0)
INCOMPLETE_MESSAGE_ID="err-06-unknown-$TEST_RUN_ID"

RESPONSE_5=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$INCOMPLETE_B64\",
      \"message_id\": \"$INCOMPLETE_MESSAGE_ID\"
    },
    \"subscription\": \"projects/test/pay.subscriptions/test\"
  }")

HTTP_5=$(echo "$RESPONSE_5" | tail -n1)
BODY_5=$(echo "$RESPONSE_5" | sed '$d')
if [[ "$HTTP_5" == "204" ]] && [[ -z "$BODY_5" ]]; then
    echo -e "  ${GREEN}✓ Unknown envelope durably accepted with HTTP 204${NC}"
else
    echo -e "  ${RED}✗ Expected an empty HTTP 204 response for unknown envelope${NC}"
    ALL_REJECTED="false"
fi

echo ""

# Step 4: Verify no database state change
echo -e "${YELLOW}[4/5] Verifying no database state change (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
REJECTED_WEBHOOK_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider_webhook_id IN ('err-06-missing-data', 'err-06-invalid-base64');" -t 2>/dev/null | tr -d ' ')
UNKNOWN_WEBHOOK_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider_webhook_id = '$INCOMPLETE_MESSAGE_ID' AND event_type = 'unknown';" -t 2>/dev/null | tr -d ' ')
UNKNOWN_DELIVERY_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.webhook_delivery wd JOIN pay.webhook_provider wp ON wp.id = wd.webhook_provider_id WHERE wp.app_id = '$BRIDGE_APP_ID' AND wp.provider_webhook_id = '$INCOMPLETE_MESSAGE_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$FINAL_PAYMENT_COUNT" == "$INITIAL_PAYMENT_COUNT" ]] && [[ "$REJECTED_WEBHOOK_COUNT" == "0" ]] && [[ "$UNKNOWN_WEBHOOK_COUNT" == "1" ]] && [[ "$UNKNOWN_DELIVERY_COUNT" == "1" ]]; then
    echo -e "${GREEN}✓ Business state unchanged and webhook audit state is exact${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database state changed with malformed payloads!${NC}"
fi
echo ""

# Step 5: Summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

# Determine overall test status
if [[ "$ALL_REJECTED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-06 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-06 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-06",
  "test_name": "Webhook Payload Malformed",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "failure_kind": $([[ "$TEST_STATUS" == "pass" ]] && echo "null" || echo '"behavior"'),
  "user_id": "$USER_ID",
  "results": {
    "all_malformed_rejected": $ALL_REJECTED,
    "no_db_state_change": $DB_UNCHANGED,
    "test_results": {
      "invalid_json": "$HTTP_1",
      "empty_payload": "$HTTP_2",
      "missing_data": "$HTTP_3",
      "invalid_base64": "$HTTP_4",
      "missing_notification": "$HTTP_5"
    },
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT,
    "rejected_webhook_count": $REJECTED_WEBHOOK_COUNT,
    "unknown_webhook_count": $UNKNOWN_WEBHOOK_COUNT,
    "unknown_delivery_count": $UNKNOWN_DELIVERY_COUNT
  }
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
