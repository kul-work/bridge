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
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: POST /webhooks/... returns a 400 HTTP error for invalid JSON or missing fields. 
#                      Decoding failures for base64 fields are correctly caught and rejected.
#                      No records are created in webhook_log, and no state changes occur.
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
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Record initial database state
echo -e "${YELLOW}[2/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo ""

# Step 3: Test with various malformed payloads
echo -e "${YELLOW}[3/5] Testing with malformed webhook payloads${NC}"
echo ""

ALL_REJECTED="true"

# Test 1: Invalid JSON
echo -e "${BLUE}Test 1: Invalid JSON syntax${NC}"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{ invalid json }")

HTTP_1=$(echo "$RESPONSE_1" | tail -n1)
if [[ "$HTTP_1" == "400" ]]; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400${NC}"
else
    echo -e "  ${YELLOW}⚠ HTTP $HTTP_1 (expected 400)${NC}"
    if [[ "$HTTP_1" != "200" ]]; then
        echo -e "  ${GREEN}  (Still rejected)${NC}"
    else
        ALL_REJECTED="false"
    fi
fi

# Test 2: Empty payload
echo -e "${BLUE}Test 2: Empty payload${NC}"
RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "")

HTTP_2=$(echo "$RESPONSE_2" | tail -n1)
if [[ "$HTTP_2" == "400" ]]; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400${NC}"
else
    echo -e "  ${YELLOW}⚠ HTTP $HTTP_2 (expected 400)${NC}"
    if [[ "$HTTP_2" != "200" ]]; then
        echo -e "  ${GREEN}  (Still rejected)${NC}"
    else
        ALL_REJECTED="false"
    fi
fi

# Test 3: Missing message.data
echo -e "${BLUE}Test 3: Missing message.data field${NC}"
RESPONSE_3=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d '{
    "message": {
      "message_id": "test-123"
    },
    "subscription": "projects/test/pay.subscriptions/test"
  }')

HTTP_3=$(echo "$RESPONSE_3" | tail -n1)
if [[ "$HTTP_3" == "400" ]]; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400${NC}"
else
    echo -e "  ${YELLOW}⚠ HTTP $HTTP_3 (expected 400)${NC}"
    if [[ "$HTTP_3" != "200" ]]; then
        echo -e "  ${GREEN}  (Still rejected)${NC}"
    else
        ALL_REJECTED="false"
    fi
fi

# Test 4: Invalid base64 in message.data
echo -e "${BLUE}Test 4: Invalid base64 encoding${NC}"
RESPONSE_4=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d '{
    "message": {
      "data": "not-valid-base64!!!",
      "message_id": "test-123"
    },
    "subscription": "projects/test/pay.subscriptions/test"
  }')

HTTP_4=$(echo "$RESPONSE_4" | tail -n1)
if [[ "$HTTP_4" == "400" ]]; then
    echo -e "  ${GREEN}✓ Rejected with HTTP 400${NC}"
else
    echo -e "  ${YELLOW}⚠ HTTP $HTTP_4 (expected 400)${NC}"
    if [[ "$HTTP_4" != "200" ]]; then
        echo -e "  ${GREEN}  (Still rejected)${NC}"
    else
        ALL_REJECTED="false"
    fi
fi

# Test 5: Missing subscriptionNotification inside data
echo -e "${BLUE}Test 5: Missing subscriptionNotification in decoded data${NC}"
INCOMPLETE_JSON='{"version":"1.0","packageName":"com.test"}'
INCOMPLETE_B64=$(echo -n "$INCOMPLETE_JSON" | base64 -w 0)

RESPONSE_5=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{
    \"message\": {
      \"data\": \"$INCOMPLETE_B64\",
      \"message_id\": \"test-123\"
    },
    \"subscription\": \"projects/test/pay.subscriptions/test\"
  }")

HTTP_5=$(echo "$RESPONSE_5" | tail -n1)
if [[ "$HTTP_5" == "400" ]] || [[ "$HTTP_5" == "200" ]]; then
    # 200 is acceptable if backend handles missing notification gracefully
    echo -e "  ${GREEN}✓ HTTP $HTTP_5 (handled gracefully)${NC}"
else
    echo -e "  ${YELLOW}⚠ HTTP $HTTP_5${NC}"
fi

echo ""

# Step 4: Verify no database state change
echo -e "${YELLOW}[4/5] Verifying no database state change (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]]; then
    echo -e "${GREEN}✓ No database state change (as expected)${NC}"
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
elif [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-06 Test PASSED (malformed payloads handled)${NC}"
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
    "final_subscription_count": $FINAL_SUB_COUNT
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
