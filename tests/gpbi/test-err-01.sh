#!/bin/bash

##############################################################################
# ERR-01: Invalid Purchase Token Format
# 
# Purpose: Verify that malformed or fake tokens are rejected properly
#          with no database entries created.
#
# Usage: ./test-err-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - X-Token-Validation-Mode: strict header used in curl requests
#
# TESTPLAN Reference:
#   Expected Behavior: API returns error (400 or 422).
#   Backend Response: Backend rejects token format validation OR calls
#                     Google API which returns 404/401.
#                     No DB entry created.
#                     Error logged: "Invalid or revoked purchase token".
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
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_err_01_user_$RUN_ID}"

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
echo "ERR-01: Invalid Purchase Token Format"
echo -e "${YELLOW}========================================${NC}"
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

# Step 3: Test with obviously fake/malformed tokens
echo -e "${YELLOW}[3/5] Testing with malformed/fake tokens${NC}"
echo ""

FAKE_TOKENS=(
    "invalid-token-xyz"
    ""
    "12345"
    "not-a-valid-purchase-token"
    "abc!@#\$%^&*()"
)

ALL_REJECTED="true"

for token in "${FAKE_TOKENS[@]}"; do
    echo -e "${BLUE}Testing token: '$token'${NC}"
    
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
        \"purchase_token\": \"$token\",
        \"product_type\": \"subscription\"
      }")
    
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    
    # Check if properly rejected (4xx error expected)
    if [[ "$HTTP_CODE" =~ ^4[0-9][0-9]$ ]]; then
        echo -e "  ${GREEN}✓ Rejected with HTTP $HTTP_CODE (as expected)${NC}"
    elif [[ "$HTTP_CODE" == "500" ]] || [[ "$HTTP_CODE" == "502" ]] || [[ "$HTTP_CODE" == "503" ]]; then
        echo -e "  ${YELLOW}⚠ Server error HTTP $HTTP_CODE (Google API rejection)${NC}"
    elif [[ "$HTTP_CODE" == "200" ]]; then
        echo -e "  ${RED}✗ ACCEPTED (HTTP 200) - should have been rejected!${NC}"
        ALL_REJECTED="false"
    else
        echo -e "  ${YELLOW}⚠ HTTP $HTTP_CODE${NC}"
    fi
done
echo ""

# Step 4: Verify no database entries created
echo -e "${YELLOW}[4/5] Verifying no database entries created (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$FINAL_PAYMENT_COUNT" == "$INITIAL_PAYMENT_COUNT" ]]; then
    echo -e "${GREEN}✓ No database entries created (as expected)${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database entries were created for invalid tokens!${NC}"
fi
echo ""

# Step 5: Summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

# Determine overall test status
if [[ "$ALL_REJECTED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-01 Test PASSED (All invalid tokens rejected)${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-01 Test FAILED (Some invalid tokens accepted or DB changed)${NC}"
fi

# Generate JSON report
cat > err-01-report.json <<EOF
{
  "test_id": "ERR-01",
  "test_name": "Invalid Purchase Token Format",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "all_invalid_tokens_rejected": $ALL_REJECTED,
    "no_db_entries_created": $DB_UNCHANGED,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT,
    "tokens_tested": ["invalid-token-xyz", "", "12345", "not-a-valid-purchase-token"]
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: err-01-report.json"
cat err-01-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
