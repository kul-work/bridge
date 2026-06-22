#!/bin/bash

##############################################################################
# ISO-01: Cross-App Subscription Visibility Isolation
#
# Purpose: Verify that App B cannot query App A's subscription data through
#          the Bridge API. RLS on pay.subscriptions must enforce app_id scoping.
#
# Usage: ./test-iso-01.sh
#
# Prerequisites:
#   - Bridge running with MOCK_EXTERNAL_APIS=true
#   - Two apps registered in pay.apps (APP_A and APP_B with different app_id)
#   - globals.cfg sourced with APP_A_ID, APP_B_ID, APP_A_API_KEY, APP_B_API_KEY
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   ISO-01: Subscription data is not visible across app boundaries (RLS enforced).
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
TEST_RUN_ID="iso-01-${TIMESTAMP}-$$"
REPORT_FILE="iso-01-report.json"
USER_ID="test_iso_01_user_$TEST_RUN_ID"
PURCHASE_TOKEN="test-iso-01-token-$TEST_RUN_ID"
SUBSCRIPTION_ID="$PRODUCT_ID_SUB"

echo -e "${YELLOW}========================================${NC}"
echo "ISO-01: Cross-App Subscription Visibility Isolation"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo "App A: $APP_A_SLUG ($APP_A_ID)"
echo "App B: $APP_B_SLUG ($APP_B_ID)"
echo ""

if [[ -z "$APP_A_ID" ]] || [[ -z "$APP_B_ID" ]]; then
    echo -e "${RED}✗ Both APP_A_ID and APP_B_ID must be set. Ensure both apps exist in pay.apps.${NC}"
    exit 1
fi

if [[ "$APP_A_ID" == "$APP_B_ID" ]]; then
    echo -e "${RED}✗ APP_A_ID and APP_B_ID must be different for isolation testing.${NC}"
    exit 1
fi

if [[ -z "${APP_A_API_KEY:-}" ]]; then
    echo -e "${RED}✗ APP_A_API_KEY is not set. Add it to .env (cannot be fetched from DB — keys are hashed).${NC}"
    exit 1
fi

if [[ -z "${APP_B_API_KEY:-}" ]]; then
    echo -e "${RED}✗ APP_B_API_KEY is not set. Add the $APP_B_SLUG API key to .env (cannot be fetched from DB — keys are hashed).${NC}"
    echo -e "${YELLOW}  To find the key, check the app's creation output or pay.api_keys table for the app_id: $APP_B_ID${NC}"
    exit 1
fi

# Step 1: Seed subscription for App A
echo -e "${YELLOW}[1/4] Seeding subscription for App A${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, purchase_token, status, auto_renewing, current_period_end)
   VALUES ('$APP_A_ID', '$USER_ID', '$SUBSCRIPTION_ID', 'google_play', '$PURCHASE_TOKEN', 'active', true, NOW() + INTERVAL '30 days');" 2>/dev/null

echo -e "${GREEN}✓ Created subscription for App A (user: $USER_ID)${NC}"
echo ""

# Step 2: Query subscription-status using App A's API key (should see it)
echo -e "${YELLOW}[2/4] Querying subscription-status with App A API key${NC}"

RESPONSE_A=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $APP_A_API_KEY" 2>/dev/null || echo "error")

HTTP_A=$(echo "$RESPONSE_A" | tail -n1)
BODY_A=$(echo "$RESPONSE_A" | sed '$d')

echo -e "${BLUE}  App A HTTP: $HTTP_A${NC}"
echo -e "${BLUE}  App A Body: ${BODY_A:0:120}...${NC}"

if [[ "$HTTP_A" != "200" ]]; then
    echo -e "${RED}✗ App A should have received 200, got $HTTP_A${NC}"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
      "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
    exit 1
fi
echo -e "${GREEN}✓ App A can see the subscription${NC}"
echo ""

# Step 3: Query subscription-status using App B's API key (should NOT see it)
echo -e "${YELLOW}[3/4] Querying subscription-status with App B API key${NC}"

RESPONSE_B=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $APP_B_API_KEY" 2>/dev/null || echo "error")

HTTP_B=$(echo "$RESPONSE_B" | tail -n1)
BODY_B=$(echo "$RESPONSE_B" | sed '$d')

echo -e "${BLUE}  App B HTTP: $HTTP_B${NC}"
echo -e "${BLUE}  App B Body: ${BODY_B:0:120}...${NC}"

ISOLATION_OK="false"
if [[ "$HTTP_B" == "404" ]] || [[ "$HTTP_B" == "200" ]]; then
    # 404 = not found (RLS blocked), 200 with empty/no subscription data = also acceptable
    if [[ "$HTTP_B" == "404" ]]; then
        echo -e "${GREEN}✓ App B received 404 (subscription invisible via RLS)${NC}"
        ISOLATION_OK="true"
    elif echo "$BODY_B" | grep -q '"is_premium":false' 2>/dev/null; then
        echo -e "${GREEN}✓ App B received 200 but with no premium data (RLS blocked)${NC}"
        ISOLATION_OK="true"
    else
        echo -e "${RED}✗ App B may have received App A's subscription data${NC}"
        echo "$BODY_B"
    fi
else
    echo -e "${RED}✗ App B received unexpected HTTP $HTTP_B${NC}"
    echo "$BODY_B"
fi
echo ""

# Step 4: Cleanup
echo -e "${YELLOW}[4/4] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine test status
if [[ "$ISOLATION_OK" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ISO-01 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ISO-01 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ISO-01",
  "test_name": "Cross-App Subscription Visibility Isolation",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "app_a": "$APP_A_SLUG ($APP_A_ID)",
  "app_b": "$APP_B_SLUG ($APP_B_ID)",
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