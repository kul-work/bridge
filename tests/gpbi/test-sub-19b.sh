#!/bin/bash

##############################################################################
# SUB-19B: LinkingRequired Response (Account Conflict)
# 
# Purpose: Test backend handling of external_account_identifiers hash mismatch 
#          when User 2 attempts to verify User 1's purchase token.
#
# Usage: ./test-sub-19b.sh
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
#   Expected Behavior: User 2 verification attempt triggers 'LinkingRequired' error.
#                      Backend detects that the token is already bound to a different external_user_id.
#                      DB state remains unchanged (User 1 remains primary owner).
#                      Ensures protection against token takeover and enforces explicit re-binding policies.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="sub-19b-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="resubscribe-linking-required-$TEST_RUN_ID"
REPORT_FILE="sub-19b-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-19B: LinkingRequired Response"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User IDs
USER1_ID="test_sub_user_19b_owner_$TEST_RUN_ID"
USER2_ID="test_sub_user_19b_competitor_$TEST_RUN_ID"
REGISTER_HTTP_CODE=0
VERIFY_HTTP_CODE=0
USER2_REGISTER_HTTP_CODE=0
USER2_VERIFY_HTTP_CODE=0

fail_test() {
    local failure_step="$1"
    local details="$2"
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-19B",
  "test_name": "LinkingRequired Response (Different Account Verification)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "user1_id": "$USER1_ID",
  "user2_id": "$USER2_ID",
  "product_id": "$PRODUCT_ID",
  "failure_step": "$failure_step",
  "details": "$details",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "user2_register_http_code": $USER2_REGISTER_HTTP_CODE,
  "user2_verify_http_code": $USER2_VERIFY_HTTP_CODE
}
EOF
    echo -e "${RED}SUB-19B failed at $failure_step: $details${NC}"
    echo "Report saved to: $REPORT_FILE"
    exit 1
}
echo -e "${GREEN}✓ Testing with User IDs: $USER1_ID (Owner), $USER2_ID (Competitor)${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id IN ('$USER1_ID', '$USER2_ID');" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id IN ('$USER1_ID', '$USER2_ID');" 2>/dev/null || true
echo ""

# Step 3: User 1 Verification (Initial owner)
echo -e "${YELLOW}[1/5] User 1 performs verification (becomes owner)${NC}"

# Pre-register
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER1_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-19b-u1\"
  }")

if [[ "$REGISTER_HTTP_CODE" != "200" ]]; then
    fail_test "user1_register" "expected HTTP 200, got $REGISTER_HTTP_CODE"
fi

# Verify
# Mock returns a fixed external_account_identifier for this specific token string in some backend versions
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER1_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

if [[ "$VERIFY_HTTP_CODE" != "200" ]]; then
    fail_test "user1_verify" "expected HTTP 200, got $VERIFY_HTTP_CODE"
fi

echo -e "${GREEN}✓ User 1 verification complete${NC}"
echo ""

# Step 4: User 2 attempts to verify the SAME token
echo -e "${YELLOW}[2/5] User 2 attempts verification (conflict expected)${NC}"

# Pre-register for User 2
USER2_REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER2_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-19b-u2\"
  }")

if [[ "$USER2_REGISTER_HTTP_CODE" != "200" ]]; then
    fail_test "user2_register" "expected HTTP 200, got $USER2_REGISTER_HTTP_CODE"
fi

# Verify (expecting LinkingRequired)
USER2_RESPONSE_WITH_CODE=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER2_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")
USER2_VERIFY_HTTP_CODE=$(echo "$USER2_RESPONSE_WITH_CODE" | tail -n1)
USER2_RESPONSE=$(echo "$USER2_RESPONSE_WITH_CODE" | sed '$d')

echo "Response: $USER2_RESPONSE"

# Step 5: Validate Response Content
echo ""
echo -e "${YELLOW}[3/5] Validating LinkingRequired response${NC}"

if [[ "$USER2_VERIFY_HTTP_CODE" != "200" ]]; then
    fail_test "user2_verify" "expected HTTP 200 linking_required response, got $USER2_VERIFY_HTTP_CODE"
elif echo "$USER2_RESPONSE" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"linking_required"'; then
    echo -e "${GREEN}✓ Success: Response status is 'linking_required'${NC}"
else
    fail_test "user2_verify" "HTTP 200 response did not contain status linking_required"
fi
echo ""

# Step 6: Verify no subscription for User 2 in DB
echo -e "${YELLOW}[4/5] Verifying User 2 has no subscription in Bridge DB${NC}"
export PGPASSWORD="postgres"
SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER2_ID';" -t | tr -d '[:space:]')

if [[ "$SUB_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ Success: User 2 has 0 subscriptions${NC}"
else
    fail_test "user2_no_subscription" "expected 0 subscriptions, got $SUB_COUNT"
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-19B",
  "test_name": "LinkingRequired Response (Different Account Verification)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user1_id": "$USER1_ID",
  "user2_id": "$USER2_ID",
  "product_id": "$PRODUCT_ID",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "user2_register_http_code": $USER2_REGISTER_HTTP_CODE,
  "user2_verify_http_code": $USER2_VERIFY_HTTP_CODE,
  "results": {
    "user1_verified": true,
    "user2_linking_required": true,
    "user2_no_subscription": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-19B Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
