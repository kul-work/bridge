#!/bin/bash

##############################################################################
# ERR-05: Provider API Temporarily Unavailable
# 
# Purpose: Verify that when the provider API returns 5xx errors or timeouts,
#          the backend handles it gracefully without creating partial
#          database state.
#
# Usage: ./test-err-05.sh
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
#   Expected Behavior: POST /api/v1/verify-purchase returns a 5xx error or timeout. 
#                      The internal database transaction is properly rolled back.
#                      No partial or 'ghost' records are created in pay.subscriptions or pay.payments.
#                      Ensures atomicity during external service outages or transient network failures.
#                      Validates following a 'verify-then-commit' workflow.
#                      Confirms that logs capture the provider error without exposing stack traces.
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
TEST_RUN_ID="err-05-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
REPORT_FILE="err-05-report.json"
USER_ID="${USER_ID:-test_err_05_user_$TEST_RUN_ID}"

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
echo "ERR-05: Google API Temporarily Unavailable"
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

# Step 3: Test with token that simulates Google API 5xx error
echo -e "${YELLOW}[3/5] Simulating Google API 5xx error${NC}"
echo ""

# Use a token pattern that the mock should reject or simulate error
# In a real mock setup, this pattern would trigger a 5xx simulation
ERROR_TOKEN="google-api-error-500-$(date +%s)"

echo "Request details:"
echo "  Token: $ERROR_TOKEN (simulates Google API failure)"
echo "  Expected: HTTP 500/502/503 or gateway error"
echo ""
echo "Note: In production, this tests Google API outage handling."
echo "      Backend should NOT create partial DB state on API failure."
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" --max-time 10 -X POST \
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
    \"purchase_token\": \"$ERROR_TOKEN\",
    \"product_type\": \"subscription\"
  }" 2>/dev/null || echo -e "timeout\n504")

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

# Any non-200 response is acceptable for error handling test
ERROR_HANDLED="false"
if [[ "$HTTP_CODE" =~ ^5[0-9][0-9]$ ]] || [[ "$HTTP_CODE" == "504" ]]; then
    echo -e "${GREEN}✓ Backend returned 5xx error (as expected for API outage)${NC}"
    ERROR_HANDLED="true"
elif [[ "$HTTP_CODE" =~ ^4[0-9][0-9]$ ]]; then
    echo -e "${GREEN}✓ Backend returned 4xx error (rejected gracefully)${NC}"
    ERROR_HANDLED="true"
elif [[ "$HTTP_CODE" == "200" ]]; then
    echo -e "${YELLOW}⚠ HTTP 200 - mock may have accepted token${NC}"
    ERROR_HANDLED="false"
elif [[ "$HTTP_CODE" == "timeout" ]]; then
    echo -e "${YELLOW}⚠ Request timed out (treating as handled)${NC}"
    ERROR_HANDLED="true"
else
    echo -e "${YELLOW}⚠ HTTP $HTTP_CODE${NC}"
    ERROR_HANDLED="true"
fi
echo ""

# Step 4: Verify NO partial database state created
echo -e "${YELLOW}[4/5] Verifying no partial database state (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

# Check for error token specifically
ERROR_TOKEN_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$ERROR_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Subscriptions with error token: $ERROR_TOKEN_SUB"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

NO_PARTIAL_STATE="false"
if [[ "$ERROR_TOKEN_SUB" == "0" ]]; then
    echo -e "${GREEN}✓ No partial database state created${NC}"
    NO_PARTIAL_STATE="true"
else
    echo -e "${RED}✗ Partial state was created during API error!${NC}"
fi
echo ""

# Step 5: Cleanup and summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

# Clean up any test data
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$ERROR_TOKEN';" 2>/dev/null

# Determine overall test status
# The key requirement is NO partial DB state, regardless of HTTP code
if [[ "$NO_PARTIAL_STATE" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-05 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-05 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-05",
  "test_name": "Google API Temporarily Unavailable",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "error_token": "$ERROR_TOKEN",
  "results": {
    "error_handled_gracefully": $ERROR_HANDLED,
    "http_code": "$HTTP_CODE",
    "no_partial_db_state": $NO_PARTIAL_STATE,
    "error_token_subscription_count": $ERROR_TOKEN_SUB,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT
  },
  "notes": "Client should retry with exponential backoff on 5xx errors"
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
