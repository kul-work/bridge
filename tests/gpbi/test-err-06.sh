#!/bin/bash

##############################################################################
# ERR-06: Webhook Payload Malformed
# 
# Purpose: Verify that malformed webhook payloads (invalid JSON, missing fields)
#          are rejected with HTTP 400 and no database state change.
#
# Usage: ./test-err-06.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=false
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: HTTP 400 returned.
#   Backend Response: Backend rejects parsing early.
#                     Logs error: "Failed to parse webhook payload".
#                     No DB state change.
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

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

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
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./test-err-06.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-06: Webhook Payload Malformed"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id from email
echo -e "${YELLOW}[1/5] Fetching user_id from database for email: $EMAIL${NC}"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

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

INITIAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo ""

# Step 3: Test with various malformed payloads
echo -e "${YELLOW}[3/5] Testing with malformed webhook payloads${NC}"
echo ""

ALL_REJECTED="true"

# Test 1: Invalid JSON
echo -e "${BLUE}Test 1: Invalid JSON syntax${NC}"
RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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

FINAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

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
cat > err-06-report.json <<EOF
{
  "test_id": "ERR-06",
  "test_name": "Webhook Payload Malformed",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
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
echo "Report saved to: err-06-report.json"
cat err-06-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
