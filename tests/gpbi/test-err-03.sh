#!/bin/bash

##############################################################################
# ERR-03: Expired Purchase Token
# 
# Purpose: Verify that expired purchase tokens (60+ days old) are rejected
#          with no database entries created.
#
# Usage: ./test-err-03.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: POST /api/v1/verify-purchase returns HTTP 502 provider_error for the mock provider's expired-token response.
#                      No database records are created or updated for expired tokens.
#                      Ensures legacy or discarded tokens cannot be used to gain fraudulent access.
#                      Validates strict compliance with provider token retention policies.
#                      Confirms that the backend correctly identifies 'Gone' status from the provider.
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
TEST_RUN_ID="err-03-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
REPORT_FILE="err-03-report.json"
USER_ID="${USER_ID:-test_err_03_user_$TEST_RUN_ID}"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
EXPIRED_TOKEN=""
HTTP_CODE=0
CURRENT_FAILURE_KIND="setup"
rm -f "$REPORT_FILE"

write_early_failure_report() {
    local exit_code=$?
    if [[ "$exit_code" -eq 0 || -f "$REPORT_FILE" ]]; then
        return
    fi
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-03",
  "test_name": "Expired Purchase Token",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "failure_kind": "$CURRENT_FAILURE_KIND",
  "failure_step": "script_exit",
  "exit_code": $exit_code,
  "http_code": $HTTP_CODE
}
EOF
}
trap write_early_failure_report EXIT

# Extract DB password once
# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-03: Expired Purchase Token"
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
INITIAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo -e "${BLUE}Initial payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo ""

# Step 3: Test with simulated expired token
CURRENT_FAILURE_KIND="behavior"
echo -e "${YELLOW}[3/5] Testing with simulated expired token${NC}"
echo ""

# Use a token that indicates it's expired (the mock should recognize this pattern)
# In real scenario, this would be a token from 60+ days ago
EXPIRED_TOKEN="mock-google-play-subscription:$PRODUCT_ID:expired-token-err-03-$(date +%s)"

echo "Request details:"
echo "  Token: $EXPIRED_TOKEN (simulating 60+ day old token)"
echo "  Expected: Error - Purchase token expired"
echo ""
echo "Note: In production, Google API rejects tokens > 60 days post-expiry."
echo "      This test simulates that behavior with mock API."
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "Content-Type: application/json" \
   \
   \
  -H "X-Token-Validation-Mode: strict" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$EXPIRED_TOKEN\",
    \"product_type\": \"subscription\"
  }")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
LINE_COUNT=$(echo "$RESPONSE" | wc -l)
BODY=""
if [ "$LINE_COUNT" -gt 1 ]; then
    BODY=$(echo "$RESPONSE" | head -n $((LINE_COUNT - 1)))
fi

echo "Response Code: $HTTP_CODE"
if [[ ! -z "$BODY" ]]; then
    echo "Response Body: $BODY"
fi
echo ""

EXPIRED_REJECTED="false"
if [[ "$HTTP_CODE" == "502" ]] && echo "$BODY" | grep -Eq '"error"[[:space:]]*:[[:space:]]*"provider_error"'; then
    echo -e "${GREEN}✓ Expired token rejected with HTTP 502 provider_error${NC}"
    EXPIRED_REJECTED="true"
else
    echo -e "${RED}✗ Expected HTTP 502 provider_error, got HTTP $HTTP_CODE${NC}"
fi
echo ""

# Step 4: Verify no database entries created for expired token
echo -e "${YELLOW}[4/5] Verifying no database entries created (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

# Check for expired token specifically
EXPIRED_TOKEN_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$EXPIRED_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Subscriptions with expired token: $EXPIRED_TOKEN_SUB"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

DB_SAFE="false"
if [[ "$EXPIRED_TOKEN_SUB" == "0" && "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" && "$FINAL_PAYMENT_COUNT" == "$INITIAL_PAYMENT_COUNT" ]]; then
    echo -e "${GREEN}✓ No subscription or payment created for expired token${NC}"
    DB_SAFE="true"
else
    echo -e "${RED}✗ Database state changed for expired token${NC}"
fi
echo ""

# Step 5: Cleanup and summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

# Clean up any test data
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$EXPIRED_TOKEN';" 2>/dev/null

# Determine overall test status
if [[ "$EXPIRED_REJECTED" == "true" ]] && [[ "$DB_SAFE" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-03 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-03 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-03",
  "test_name": "Expired Purchase Token",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "failure_kind": $([[ "$TEST_STATUS" == "pass" ]] && echo "null" || echo '"behavior"'),
  "user_id": "$USER_ID",
  "expired_token": "$EXPIRED_TOKEN",
  "results": {
    "expired_token_rejected": $EXPIRED_REJECTED,
    "http_code": $HTTP_CODE,
    "no_db_entries_created": $DB_SAFE,
    "expired_token_subscription_count": $EXPIRED_TOKEN_SUB,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT
  },
  "notes": "Purchase tokens expire 60 days after subscription expiration"
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
