#!/bin/bash

##############################################################################
# ISO-02: Cross-App Payment History Isolation
#
# Purpose: Verify that App B cannot query App A's payment records through
#          the Bridge API. RLS on pay.payments must enforce app_id scoping.
#
# Usage: ./test-iso-02.sh
#
# TESTPLAN Reference:
#   ISO-02: Payment history is not visible across app boundaries (RLS enforced).
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
TEST_RUN_ID="iso-02-${TIMESTAMP}-$$"
REPORT_FILE="iso-02-report.json"
USER_ID="test_iso_02_user_$TEST_RUN_ID"
PROVIDER_TXN_ID="test-iso-02-txn-$TEST_RUN_ID"
SUBSCRIPTION_ID="$PRODUCT_ID_SUB"

echo -e "${YELLOW}========================================${NC}"
echo "ISO-02: Cross-App Payment History Isolation"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if [[ -z "$APP_A_ID" ]] || [[ -z "$APP_B_ID" ]]; then
    echo -e "${RED}✗ Both APP_A_ID and APP_B_ID must be set.${NC}"
    exit 1
fi

if [[ -z "${APP_A_API_KEY:-}" ]]; then
    echo -e "${RED}✗ APP_A_API_KEY is not set. Add it to .env (cannot be fetched from DB — keys are hashed).${NC}"
    exit 1
fi

if [[ -z "${APP_B_API_KEY:-}" ]]; then
    echo -e "${RED}✗ APP_B_API_KEY is not set. Add the $APP_B_SLUG API key to .env (cannot be fetched from DB — keys are hashed).${NC}"
    exit 1
fi

# Step 1: Seed payment for App A
echo -e "${YELLOW}[1/4] Seeding payment record for App A${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.payments WHERE provider_transaction_id = '$PROVIDER_TXN_ID';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.payments (app_id, external_user_id, provider, provider_transaction_id, subscription_id, product_id, amount_cents, currency, status)
   VALUES ('$APP_A_ID', '$USER_ID', 'google_play', '$PROVIDER_TXN_ID', '$SUBSCRIPTION_ID', '$SUBSCRIPTION_ID', 999, 'USD', 'success');" 2>/dev/null

echo -e "${GREEN}✓ Created payment for App A (user: $USER_ID, txn: $PROVIDER_TXN_ID)${NC}"
echo ""

# Step 2: Query payments using App A's API key (should see it)
echo -e "${YELLOW}[2/4] Querying payments with App A API key${NC}"

RESPONSE_A=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/payments?external_user_id=$USER_ID" \
  -H "Authorization: Bearer $APP_A_API_KEY" 2>/dev/null || echo "error")

HTTP_A=$(echo "$RESPONSE_A" | tail -n1)
BODY_A=$(echo "$RESPONSE_A" | sed '$d')

echo -e "${BLUE}  App A HTTP: $HTTP_A${NC}"

if [[ "$HTTP_A" != "200" ]]; then
    echo -e "${RED}✗ App A should have received 200, got $HTTP_A${NC}"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
      "DELETE FROM pay.payments WHERE provider_transaction_id = '$PROVIDER_TXN_ID';" 2>/dev/null
    exit 1
fi

# Check that App A can see the payment
if echo "$BODY_A" | grep -q "$PROVIDER_TXN_ID" 2>/dev/null; then
    echo -e "${GREEN}✓ App A can see the payment${NC}"
else
    echo -e "${YELLOW}⚠ App A got 200 but payment not found in response body (may be empty list)${NC}"
fi
echo ""

# Step 3: Query payments using App B's API key (should NOT see App A's payments)
echo -e "${YELLOW}[3/4] Querying payments with App B API key${NC}"

RESPONSE_B=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/payments?external_user_id=$USER_ID" \
  -H "Authorization: Bearer $APP_B_API_KEY" 2>/dev/null || echo "error")

HTTP_B=$(echo "$RESPONSE_B" | tail -n1)
BODY_B=$(echo "$RESPONSE_B" | sed '$d')

echo -e "${BLUE}  App B HTTP: $HTTP_B${NC}"

ISOLATION_OK="false"
if [[ "$HTTP_B" == "200" ]]; then
    if echo "$BODY_B" | grep -q "$PROVIDER_TXN_ID" 2>/dev/null; then
        echo -e "${RED}✗ App B can see App A's payment (RLS breach!)${NC}"
        echo "$BODY_B"
    else
        echo -e "${GREEN}✓ App B received 200 but payment is NOT visible (RLS enforced)${NC}"
        ISOLATION_OK="true"
    fi
elif [[ "$HTTP_B" == "404" ]]; then
    echo -e "${GREEN}✓ App B received 404 (payment invisible via RLS)${NC}"
    ISOLATION_OK="true"
else
    echo -e "${RED}✗ App B received unexpected HTTP $HTTP_B${NC}"
    echo "$BODY_B"
fi
echo ""

# Step 4: Cleanup
echo -e "${YELLOW}[4/4] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.payments WHERE provider_transaction_id = '$PROVIDER_TXN_ID';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

if [[ "$ISOLATION_OK" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ISO-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ISO-02 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ISO-02",
  "test_name": "Cross-App Payment History Isolation",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "provider_transaction_id": "$PROVIDER_TXN_ID",
  "results": {
    "app_a_http": "$HTTP_A",
    "app_b_http": "$HTTP_B",
    "isolation_enforced": $ISOLATION_OK
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0