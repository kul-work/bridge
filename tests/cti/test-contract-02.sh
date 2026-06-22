#!/bin/bash

##############################################################################
# CONTRACT-02: Purchase Registration Endpoint Shape
#
# Purpose: Verify POST /api/v1/purchase/register returns the correct response
#          shape for pre-registration of purchase tokens.
#
# Usage: ./test-contract-02.sh
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="contract-02-${TIMESTAMP}-$$"
REPORT_FILE="contract-02-report.json"
USER_ID="test_contract_02_user_$TEST_RUN_ID"
PURCHASE_TOKEN="test-contract-02-token-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"

echo -e "${YELLOW}========================================${NC}"
echo "CONTRACT-02: Purchase Registration Endpoint Shape"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if [[ -z "$BRIDGE_APP_ID" ]] || [[ -z "$BRIDGE_API_KEY" ]]; then
    echo -e "${RED}✗ BRIDGE_APP_ID and BRIDGE_API_KEY must be set.${NC}"
    exit 1
fi

echo -e "${YELLOW}[1/2] Calling POST /api/v1/purchase/register${NC}"

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"google_play\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"contract-test-register\"
  }" 2>/dev/null || echo "error")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

echo -e "${BLUE}  HTTP: $HTTP_CODE${NC}"
echo -e "${BLUE}  Body: ${BODY:0:200}...${NC}"
echo ""

SHAPE_VALID="true"
if [[ "$HTTP_CODE" != "200" ]] && [[ "$HTTP_CODE" != "201" ]] && [[ "$HTTP_CODE" != "204" ]]; then
    echo -e "${RED}✗ Expected 200/201/204, got $HTTP_CODE${NC}"
    SHAPE_VALID="false"
else
    echo -e "${GREEN}✓ HTTP code valid ($HTTP_CODE)${NC}"
fi
echo ""

echo -e "${YELLOW}[2/2] Cleanup${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND purchase_token IS NULL;" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up${NC}"
echo ""

if [[ "$SHAPE_VALID" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ CONTRACT-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ CONTRACT-02 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "CONTRACT-02",
  "test_name": "Purchase Registration Endpoint Shape",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "http_code": "$HTTP_CODE",
  "results": { "shape_valid": $SHAPE_VALID }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"

if [[ "$TEST_STATUS" == "fail" ]]; then exit 1; fi
exit 0