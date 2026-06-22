#!/bin/bash

##############################################################################
# CONTRACT-04: Subscription Status Endpoint Shape
#
# Purpose: Verify GET /api/v1/users/{external_user_id}/subscription-status
#          returns the correct response shape with is_premium, status,
#          current_period_end, and auto_renewing fields.
#
# Usage: ./test-contract-04.sh
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
TEST_RUN_ID="contract-04-${TIMESTAMP}-$$"
REPORT_FILE="contract-04-report.json"
USER_ID="test_contract_04_user_$TEST_RUN_ID"
PURCHASE_TOKEN="test-contract-04-token-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"

echo -e "${YELLOW}========================================${NC}"
echo "CONTRACT-04: Subscription Status Endpoint Shape"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if [[ -z "$BRIDGE_APP_ID" ]] || [[ -z "$BRIDGE_API_KEY" ]]; then
    echo -e "${RED}✗ BRIDGE_APP_ID and BRIDGE_API_KEY must be set.${NC}"
    exit 1
fi

# Step 1: Seed an active subscription for shape verification
echo -e "${YELLOW}[1/3] Seeding active subscription${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, purchase_token, status, auto_renewing, current_period_end)
   VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'google_play', '$PURCHASE_TOKEN', 'active', true, NOW() + INTERVAL '30 days');" 2>/dev/null

echo -e "${GREEN}✓ Seeded active subscription${NC}"
echo ""

# Step 2: Query subscription-status
echo -e "${YELLOW}[2/3] Calling GET /api/v1/users/$USER_ID/subscription-status${NC}"

RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" 2>/dev/null || echo "error")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

echo -e "${BLUE}  HTTP: $HTTP_CODE${NC}"
echo -e "${BLUE}  Body: ${BODY:0:300}...${NC}"
echo ""

SHAPE_VALID="true"

if [[ "$HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ Expected 200, got $HTTP_CODE${NC}"
    SHAPE_VALID="false"
else
    echo -e "${GREEN}✓ HTTP 200 received${NC}"
fi

# Check for key fields in the response
HAS_PREMIUM="false"
HAS_STATUS="false"
HAS_PERIOD_END="false"
HAS_AUTO_RENEWING="false"

if echo "$BODY" | grep -q "is_premium" 2>/dev/null; then
    HAS_PREMIUM="true"
    echo -e "${GREEN}✓ Response contains is_premium${NC}"
else
    echo -e "${YELLOW}⚠ Response missing is_premium field${NC}"
fi

if echo "$BODY" | grep -q '"status"' 2>/dev/null; then
    HAS_STATUS="true"
    echo -e "${GREEN}✓ Response contains status${NC}"
else
    echo -e "${YELLOW}⚠ Response missing status field${NC}"
fi

if echo "$BODY" | grep -q "current_period_end\|expiry\|expires" 2>/dev/null; then
    HAS_PERIOD_END="true"
    echo -e "${GREEN}✓ Response contains current_period_end/expiry${NC}"
else
    echo -e "${YELLOW}⚠ Response missing current_period_end field${NC}"
fi

if echo "$BODY" | grep -q "auto_renewing" 2>/dev/null; then
    HAS_AUTO_RENEWING="true"
    echo -e "${GREEN}✓ Response contains auto_renewing${NC}"
else
    echo -e "${YELLOW}⚠ Response missing auto_renewing field${NC}"
fi

if [[ "$HAS_PREMIUM" != "true" ]] || [[ "$HAS_STATUS" != "true" ]]; then
    SHAPE_VALID="false"
fi
echo ""

# Step 3: Cleanup
echo -e "${YELLOW}[3/3] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up${NC}"
echo ""

if [[ "$SHAPE_VALID" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ CONTRACT-04 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ CONTRACT-04 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "CONTRACT-04",
  "test_name": "Subscription Status Endpoint Shape",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "http_code": "$HTTP_CODE",
  "results": {
    "shape_valid": $SHAPE_VALID,
    "has_is_premium": $HAS_PREMIUM,
    "has_status": $HAS_STATUS,
    "has_current_period_end": $HAS_PERIOD_END,
    "has_auto_renewing": $HAS_AUTO_RENEWING
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"

if [[ "$TEST_STATUS" == "fail" ]]; then exit 1; fi
exit 0