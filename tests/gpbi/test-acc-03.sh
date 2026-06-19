#!/bin/bash

##############################################################################
# ACC-03: Token Uniqueness & Fraud Prevention
#
# Purpose: Verify purchase token cannot be verified by different users.
#
# Usage: ./test-acc-03.sh
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
#   Expected Behavior: User A successfully verifies a token (HTTP 200).
#                      User B attempts verification and is REJECTED (LinkingRequired).
#                      Database enforces unique binding of (token, provider).
#                      Prevents token stealing/reuse across accounts.
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
TEST_RUN_ID="acc-03-${TIMESTAMP}-$$"
USER_A_ID="test_acc_03_user_a_$TEST_RUN_ID"
USER_B_ID="test_acc_03_user_b_$TEST_RUN_ID"
PURCHASE_TOKEN="test-acc-03-token-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="acc-03-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "ACC-03: Token Uniqueness & Fraud Prevention"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""
echo -e "${GREEN}Testing with generated User IDs: $USER_A_ID (Owner), $USER_B_ID (Competitor)${NC}"
echo ""

# Step 1: Clean up any existing test data
echo -e "${YELLOW}[1/6] Cleaning up previous test data${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id IN ('$USER_A_ID', '$USER_B_ID') OR purchase_token = '$PURCHASE_TOKEN';" \
  2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id IN ('$USER_A_ID', '$USER_B_ID') OR provider_transaction_id = '$PURCHASE_TOKEN';" \
  2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.fraud_prevention WHERE external_user_id IN ('$USER_A_ID', '$USER_B_ID') OR subscription_id = '$PRODUCT_ID';" \
  2>/dev/null || true

echo -e "${GREEN}Cleanup complete${NC}"
echo -e "${BLUE}Shared token for test: $PURCHASE_TOKEN${NC}"
echo ""

# Step 2: User A registers the purchase
echo -e "${YELLOW}[2/6] User A pre-registers the purchase${NC}"

REGISTER_HTTP_CODE_A=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_A_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-acc-03-user-a\"
  }")

echo "  Register response code: $REGISTER_HTTP_CODE_A"
echo ""

# Step 3: User A successfully verifies the purchase token
echo -e "${YELLOW}[3/6] User A verifies the purchase token${NC}"

VERIFY_RESPONSE_A=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_A_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

HTTP_CODE_A=$(echo "$VERIFY_RESPONSE_A" | tail -n1)
LINE_COUNT_A=$(echo "$VERIFY_RESPONSE_A" | wc -l)
VERIFY_BODY_A=""
if [[ "$LINE_COUNT_A" -gt 1 ]]; then
    VERIFY_BODY_A=$(echo "$VERIFY_RESPONSE_A" | head -n $((LINE_COUNT_A - 1)))
fi

echo "  Verify response code: $HTTP_CODE_A"
if [[ -n "$VERIFY_BODY_A" ]]; then
    echo "  Response body: $VERIFY_BODY_A"
fi

USER_A_SUCCESS="false"
if [[ "$HTTP_CODE_A" == "200" ]]; then
    echo -e "  ${GREEN}User A verification succeeded${NC}"
    USER_A_SUCCESS="true"
else
    echo -e "  ${RED}User A verification failed${NC}"
fi
echo ""

# Step 4: Verify User A owns the token and fraud prevention mapping is recorded in the database
echo -e "${YELLOW}[4/6] Verifying token ownership and fraud prevention in Bridge DB${NC}"

TOKEN_OWNER=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT external_user_id FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d '[:space:]')

SUB_EXISTS="false"
if [[ "$TOKEN_OWNER" == "$USER_A_ID" ]]; then
    echo -e "  ${GREEN}Token is owned by User A as expected${NC}"
    SUB_EXISTS="true"
else
    echo -e "  ${RED}Unexpected token owner: ${TOKEN_OWNER:-<none>}${NC}"
fi

FRAUD_MAPPING_USER=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT external_user_id FROM pay.fraud_prevention WHERE subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d '[:space:]')

FRAUD_RECORDED="false"
if [[ "$FRAUD_MAPPING_USER" == "$USER_A_ID" ]]; then
    echo -e "  ${GREEN}Fraud prevention mapping recorded successfully for User A${NC}"
    FRAUD_RECORDED="true"
else
    echo -e "  ${RED}Fraud prevention mapping NOT recorded or mismatch: ${FRAUD_MAPPING_USER:-<none>}${NC}"
fi
echo ""

# Step 5: User B attempts to verify the same token
echo -e "${YELLOW}[5/6] User B attempts to verify the same token${NC}"

VERIFY_RESPONSE_B=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_B_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

HTTP_CODE_B=$(echo "$VERIFY_RESPONSE_B" | tail -n1)
LINE_COUNT_B=$(echo "$VERIFY_RESPONSE_B" | wc -l)
VERIFY_BODY_B=""
if [[ "$LINE_COUNT_B" -gt 1 ]]; then
    VERIFY_BODY_B=$(echo "$VERIFY_RESPONSE_B" | head -n $((LINE_COUNT_B - 1)))
fi

echo "  Verify response code: $HTTP_CODE_B"
if [[ -n "$VERIFY_BODY_B" ]]; then
    echo "  Response body: $VERIFY_BODY_B"
fi

USER_B_REJECTED="false"
if [[ "$HTTP_CODE_B" =~ ^4[0-9][0-9]$ ]]; then
    echo -e "  ${GREEN}User B verification was rejected${NC}"
    USER_B_REJECTED="true"
elif echo "$VERIFY_BODY_B" | grep -qi "LinkingRequired\|linking_required"; then
    echo -e "  ${GREEN}User B received a linking-required rejection${NC}"
    USER_B_REJECTED="true"
elif [[ "$HTTP_CODE_B" == "200" ]]; then
    echo -e "  ${RED}Cross-account token was accepted - fraud prevention failed${NC}"
else
    echo -e "  ${YELLOW}Unexpected response code from User B verification: $HTTP_CODE_B${NC}"
fi
echo ""

# Step 6: Verify no duplicate subscription was created for User B
echo -e "${YELLOW}[6/6] Verifying no duplicate subscription for User B${NC}"

USER_B_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_B_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d '[:space:]')
TOKEN_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d '[:space:]')

echo "  User B subscription count: $USER_B_SUB_COUNT (expected: 0)"
echo "  Subscriptions with this token: $TOKEN_SUB_COUNT (expected: 1)"

NO_DUPLICATE="false"
if [[ "$USER_B_SUB_COUNT" == "0" ]] && [[ "$TOKEN_SUB_COUNT" == "1" ]]; then
    echo -e "  ${GREEN}No duplicate subscription was created${NC}"
    NO_DUPLICATE="true"
else
    echo -e "  ${RED}Duplicate subscription state detected${NC}"
fi
echo ""

# # Cleanup
# psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
#   -c "DELETE FROM pay.subscriptions WHERE external_user_id IN ('$USER_A_ID', '$USER_B_ID') OR purchase_token = '$PURCHASE_TOKEN';" \
#   2>/dev/null || true
# psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
#   -c "DELETE FROM pay.payments WHERE external_user_id IN ('$USER_A_ID', '$USER_B_ID') OR provider_transaction_id = '$PURCHASE_TOKEN';" \
#   2>/dev/null || true
# psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
#   -c "DELETE FROM pay.fraud_prevention WHERE external_user_id IN ('$USER_A_ID', '$USER_B_ID') OR subscription_id = '$PRODUCT_ID';" \
#   2>/dev/null || true
# echo -e "${BLUE}Cleaned up test data${NC}"
# echo ""

if [[ "$USER_A_SUCCESS" == "true" ]] && [[ "$SUB_EXISTS" == "true" ]] && [[ "$FRAUD_RECORDED" == "true" ]] && [[ "$USER_B_REJECTED" == "true" ]] && [[ "$NO_DUPLICATE" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}ACC-03 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}ACC-03 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ACC-03",
  "test_name": "Token Uniqueness & Fraud Prevention",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_a_id": "$USER_A_ID",
  "user_b_id": "$USER_B_ID",
  "shared_token": "$PURCHASE_TOKEN",
  "results": {
    "user_a_register_http_code": $REGISTER_HTTP_CODE_A,
    "user_a_verification_success": $USER_A_SUCCESS,
    "user_a_verify_http_code": $HTTP_CODE_A,
    "fraud_prevention_recorded": $FRAUD_RECORDED,
    "user_b_verification_rejected": $USER_B_REJECTED,
    "user_b_verify_http_code": $HTTP_CODE_B,
    "no_duplicate_subscription": $NO_DUPLICATE,
    "user_b_subscription_count": $USER_B_SUB_COUNT,
    "token_subscription_count": $TOKEN_SUB_COUNT
  },
  "notes": "Prevents purchase-token reuse across generated external_user_id values"
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
