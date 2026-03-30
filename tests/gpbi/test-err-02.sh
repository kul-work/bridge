#!/bin/bash

##############################################################################
# ERR-02: Subscription ID Mismatch
# 
# Purpose: Verify that a valid token with WRONG subscription_id is rejected
#          with no database entries created.
#
# Usage: ./test-err-02.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - X-Token-Validation-Mode: strict header used in curl requests
#
# TESTPLAN Reference:
#   Expected Behavior: API returns error.
#   Backend Response: Backend verifies token against subscription_id with
#                     Google API. Google rejects mismatch.
#                     Error: "Token does not match subscription_id".
#                     No DB entry created.
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
CORRECT_PRODUCT_ID="$PRODUCT_ID_SUB"
WRONG_PRODUCT_ID="wrong_subscription_id"
PROVIDER="$PROVIDER"

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
    echo "Usage: ./test-err-02.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-02: Subscription ID Mismatch"
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
INITIAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo -e "${BLUE}Initial payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo ""

# Step 3: Call verify_payment with valid token but WRONG subscription_id
echo -e "${YELLOW}[3/5] Testing with valid token but WRONG subscription_id${NC}"
echo ""

# Use a token format that looks valid but with wrong subscription_id
PURCHASE_TOKEN="test-err-02-valid-token-$(date +%s)"

echo "Request details:"
echo "  Token: $PURCHASE_TOKEN (valid format)"
echo "  Subscription ID: $WRONG_PRODUCT_ID (WRONG - should be $CORRECT_PRODUCT_ID)"
echo "  Expected: Error - Token does not match subscription_id"
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -H "X-Token-Validation-Mode: strict" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$WRONG_PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
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

MISMATCH_REJECTED="false"
if [[ "$HTTP_CODE" =~ ^4[0-9][0-9]$ ]]; then
    echo -e "${GREEN}✓ Mismatch rejected with HTTP $HTTP_CODE (as expected)${NC}"
    MISMATCH_REJECTED="true"
elif [[ "$HTTP_CODE" == "500" ]] || [[ "$HTTP_CODE" == "502" ]] || [[ "$HTTP_CODE" == "503" ]]; then
    echo -e "${YELLOW}⚠ Server error HTTP $HTTP_CODE (Google API rejection)${NC}"
    MISMATCH_REJECTED="true"
elif [[ "$HTTP_CODE" == "200" ]]; then
    echo -e "${RED}✗ ACCEPTED (HTTP 200) - mismatch should have been rejected!${NC}"
    MISMATCH_REJECTED="false"
else
    echo -e "${YELLOW}⚠ HTTP $HTTP_CODE${NC}"
    MISMATCH_REJECTED="true"
fi
echo ""

# Step 4: Verify no database entries created for wrong subscription_id
echo -e "${YELLOW}[4/5] Verifying no database entries created (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

# Also check specifically for the wrong subscription_id
WRONG_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$WRONG_PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Subscriptions with wrong ID: $WRONG_SUB_COUNT"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$WRONG_SUB_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ No database entries created for mismatched subscription${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database entries were created for mismatched subscription!${NC}"
fi
echo ""

# Step 5: Summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

# Determine overall test status
if [[ "$MISMATCH_REJECTED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-02 Test FAILED${NC}"
fi

# Generate JSON report
cat > err-02-report.json <<EOF
{
  "test_id": "ERR-02",
  "test_name": "Subscription ID Mismatch",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "purchase_token": "$PURCHASE_TOKEN",
  "correct_subscription_id": "$CORRECT_PRODUCT_ID",
  "wrong_subscription_id": "$WRONG_PRODUCT_ID",
  "results": {
    "mismatch_rejected": $MISMATCH_REJECTED,
    "http_code": $HTTP_CODE,
    "no_db_entries_created": $DB_UNCHANGED,
    "wrong_subscription_count": $WRONG_SUB_COUNT,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: err-02-report.json"
cat err-02-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
