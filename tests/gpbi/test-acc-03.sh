#!/bin/bash

##############################################################################
# ACC-03: Token Uniqueness & Fraud Prevention
# 
# Purpose: Verify that the same purchase token cannot be verified by two
#          different users (fraud prevention).
#
# Usage: ./test-acc-03.sh --email "user@example.com" --email2 "other@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - Two different user accounts in the database
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: User B's verification fails. Error returned:
#                      "Token already associated with another account".
#   Backend Logic: Backend maintains unique constraint (purchase_token, subscription_id) → user_id.
#                  On second verify attempt: Query DB, find existing token → different user_id,
#                  Reject verify call, return 400/403 error,
#                  Log fraud warning, Do NOT create new subscription record for user B.
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

# Defaults
EMAIL=""
EMAIL2=""
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
        --email2)
            EMAIL2="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Strip quotes from email addresses (in case they're passed with literal quotes)
EMAIL="${EMAIL%\"}"
EMAIL="${EMAIL#\"}"
EMAIL2="${EMAIL2%\"}"
EMAIL2="${EMAIL2#\"}"

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./test-acc-03.sh --email \"user@example.com\" --email2 \"other@example.com\""
    exit 1
fi

# If email2 not provided, we'll create a mock second user scenario
if [[ -z "$EMAIL2" ]]; then
    echo -e "${YELLOW}Note: --email2 not provided, will simulate with mock user B${NC}"
    EMAIL2=""
fi

echo -e "${YELLOW}========================================${NC}"
echo "ACC-03: Token Uniqueness & Fraud Prevention"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id for User A
echo -e "${YELLOW}[1/6] Fetching user_id for User A: $EMAIL${NC}"

USER_A_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT external_user_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)

if [[ -z "$USER_A_ID" ]] || [[ "$USER_A_ID" == *"error"* ]] || [[ "$USER_A_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id for User A${NC}"
    echo "Error: $USER_A_ID"
    exit 1
fi

USER_A_ID=$(echo "$USER_A_ID" | tr -d ' ')
echo -e "${GREEN}✓ User A ID: $USER_A_ID${NC}"

# Get or create User B
if [[ ! -z "$EMAIL2" ]]; then
    USER_B_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT external_user_id FROM users WHERE email = '$EMAIL2';" -t 2>&1 || true)
    if [[ -z "$USER_B_ID" ]] || [[ "$USER_B_ID" == *"error"* ]] || [[ "$USER_B_ID" == *"ERROR"* ]]; then
        echo -e "${YELLOW}⚠ User B ($EMAIL2) not found, using mock ID${NC}"
        USER_B_ID="mock-user-b-$(date +%s)"
        EMAIL2="mockuserb@test.ro"
    else
        USER_B_ID=$(echo "$USER_B_ID" | tr -d ' ')
    fi
else
    USER_B_ID="mock-user-b-$(date +%s)"
    EMAIL2="mockuserb@test.ro"
fi

echo -e "${GREEN}✓ User B ID: $USER_B_ID${NC}"
echo ""

# Step 2: Clean up any existing test data
echo -e "${YELLOW}[2/6] Cleaning up previous test data${NC}"

PURCHASE_TOKEN="test-acc-03-shared-token-$(date +%s)"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_A_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_B_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

echo -e "${GREEN}✓ Cleanup complete${NC}"
echo -e "${BLUE}Shared token for test: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: User A successfully verifies the purchase token
echo -e "${YELLOW}[3/6] User A verifies purchase token via /api/v1/verify-purchase${NC}"
echo ""

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  User: User A ($USER_A_ID)"
echo "  Token: $PURCHASE_TOKEN"
echo ""

VERIFY_RESPONSE_A=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "X-Test-User-ID: $USER_A_ID" \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

HTTP_CODE_A=$(echo "$VERIFY_RESPONSE_A" | tail -n1)
LINE_COUNT_A=$(echo "$VERIFY_RESPONSE_A" | wc -l)
VERIFY_BODY_A=""
if [ "$LINE_COUNT_A" -gt 1 ]; then
    VERIFY_BODY_A=$(echo "$VERIFY_RESPONSE_A" | head -n $((LINE_COUNT_A - 1)))
fi

echo "  Response Code: $HTTP_CODE_A"

USER_A_SUCCESS="false"
if [[ "$HTTP_CODE_A" == "200" ]]; then
    echo -e "  ${GREEN}✓ User A verification successful${NC}"
    USER_A_SUCCESS="true"
else
    echo -e "  ${RED}✗ User A verification failed (HTTP $HTTP_CODE_A)${NC}"
    echo "  Response: $VERIFY_BODY_A"
fi
echo ""

# Step 4: Verify User A's subscription exists in database
echo -e "${YELLOW}[4/6] Verifying User A's subscription in database${NC}"

USER_A_SUB=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT external_user_id, status, purchase_token FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null || echo "")

if [[ ! -z "$USER_A_SUB" ]] && [[ "$USER_A_SUB" != *"(0 rows)"* ]]; then
    echo -e "  ${GREEN}✓ Subscription record found for token${NC}"
    echo "  $USER_A_SUB"
    SUB_EXISTS="true"
else
    echo -e "  ${YELLOW}⚠ No subscription record found${NC}"
    SUB_EXISTS="false"
fi
echo ""

# Step 5: User B attempts to verify the SAME purchase token (fraud attempt - Cross-Account Token Rejection)
echo -e "${YELLOW}[5/6] User B attempts to verify SAME token (cross-account token rejection test)${NC}"
echo ""

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  User: User B ($USER_B_ID) - DIFFERENT USER"
echo "  Token: $PURCHASE_TOKEN (SAME as User A)"
echo "  Expected: HTTP 401/403 (cross-account token rejection)"
echo ""

VERIFY_RESPONSE_B=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "X-Test-User-ID: $USER_B_ID" \
  -H "X-Test-Email: $EMAIL2" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

HTTP_CODE_B=$(echo "$VERIFY_RESPONSE_B" | tail -n1)
LINE_COUNT_B=$(echo "$VERIFY_RESPONSE_B" | wc -l)
VERIFY_BODY_B=""
if [ "$LINE_COUNT_B" -gt 1 ]; then
    VERIFY_BODY_B=$(echo "$VERIFY_RESPONSE_B" | head -n $((LINE_COUNT_B - 1)))
fi

echo "  Response Code: $HTTP_CODE_B"
if [[ ! -z "$VERIFY_BODY_B" ]]; then
    echo "  Response Body: $VERIFY_BODY_B"
fi

USER_B_REJECTED="false"
if [[ "$HTTP_CODE_B" == "401" ]] || [[ "$HTTP_CODE_B" == "403" ]] || [[ "$HTTP_CODE_B" == "400" ]] || [[ "$HTTP_CODE_B" == "409" ]]; then
    echo -e "  ${GREEN}✓ User B verification correctly REJECTED (HTTP $HTTP_CODE_B)${NC}"
    USER_B_REJECTED="true"
elif [[ "$HTTP_CODE_B" == "200" ]]; then
    echo -e "  ${RED}✗ Cross-account token was accepted - FRAUD PREVENTION FAILED!${NC}"
    exit 1
else
    echo -e "  ${YELLOW}⚠ Unexpected response code: $HTTP_CODE_B${NC}"
    # Still might be a rejection
    if [[ "$HTTP_CODE_B" =~ ^4[0-9][0-9]$ ]]; then
        USER_B_REJECTED="true"
    fi
fi
echo ""

# Step 6: Verify no duplicate subscription created for User B (DB Validation)
echo -e "${YELLOW}[6/6] Verifying no duplicate subscription for User B (DB Validation)${NC}"

USER_B_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_B_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
TOKEN_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "  User B subscription count: $USER_B_SUB_COUNT (expected: 0)"
echo "  Subscriptions with this token: $TOKEN_SUB_COUNT (expected: 1)"
echo ""

NO_DUPLICATE="false"
if [[ "$USER_B_SUB_COUNT" == "0" ]] && [[ "$TOKEN_SUB_COUNT" == "1" ]]; then
    echo -e "  ${GREEN}✓ No duplicate subscription created (fraud prevented)${NC}"
    NO_DUPLICATE="true"
else
    echo -e "  ${RED}✗ Duplicate subscription may have been created!${NC}"
    NO_DUPLICATE="false"
fi
echo ""

# Cleanup
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${BLUE}Cleaned up test data${NC}"
echo ""

# Determine overall test status
if [[ "$USER_A_SUCCESS" == "true" ]] && [[ "$USER_B_REJECTED" == "true" ]] && [[ "$NO_DUPLICATE" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ACC-03 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ACC-03 Test FAILED${NC}"
fi

# Generate JSON report
cat > acc-03-report.json <<EOF
{
  "test_id": "ACC-03",
  "test_name": "Token Uniqueness & Fraud Prevention",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_a_id": "$USER_A_ID",
  "user_a_email": "$EMAIL",
  "user_b_id": "$USER_B_ID",
  "user_b_email": "$EMAIL2",
  "shared_token": "$PURCHASE_TOKEN",
  "results": {
    "user_a_verification_success": $USER_A_SUCCESS,
    "user_a_http_code": $HTTP_CODE_A,
    "user_b_verification_rejected": $USER_B_REJECTED,
    "user_b_http_code": $HTTP_CODE_B,
    "no_duplicate_subscription": $NO_DUPLICATE,
    "user_b_subscription_count": $USER_B_SUB_COUNT,
    "token_subscription_count": $TOKEN_SUB_COUNT
  },
  "notes": "Prevents token sharing across accounts (fraud prevention)"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: acc-03-report.json"
cat acc-03-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
