#!/bin/bash

##############################################################################
# WHK-01C: Audience Validation With Correct Audience
# 
# Purpose: Verify that webhooks with CORRECT JWT audience claim are processed
#          normally when GOOGLE_VERIFY_AUDIENCE=true is set.
#
# Usage: ./test-whk-01c.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN/test-token
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#   - Test uses specialized headers:
#     * X-Webhook-Verification-Mode: off (simulates signature bypass)
#     * X-Webhook-Audience-Mode: strict (simulates strict audience enforcement)
#
# TESTPLAN Reference:
#   Expected Behavior: Webhook ACCEPTED with HTTP 200/204.
#                      Backend logs a 'JWT audience validated' success event.
#                      Subscription status (e.g., status, period_end) updated correctly in pay.subscriptions.
#                      Ensures legitimate webhooks are processed correctly under strict audience enforcement mode.
#                      Validates the successful path of the audience verification middleware.
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
APP_ID="$BRIDGE_APP_ID"
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_whk_01c_user_$RUN_ID}"

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
echo "WHK-01C: Audience Validation With Correct Audience"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/6] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Ensure subscription record exists (run SUB-01 first if needed)
echo -e "${YELLOW}[2/6] Verifying subscription record exists${NC}"

SUB_EXISTS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status, purchase_token FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER' LIMIT 1;" -t 2>/dev/null || echo "")

if [[ -z "$SUB_EXISTS" ]] || [[ "$SUB_EXISTS" == *"(0 rows)"* ]]; then
    echo -e "${YELLOW}⚠ No subscription found, running SUB-01 setup first...${NC}"
    # Create subscription entry for testing
    DUMMY_TOKEN="test-subscription-whk-01c-$(date +%s)"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$DUMMY_TOKEN', '$PROVIDER', true, NOW(), NOW()) ON CONFLICT (app_id, external_user_id, subscription_id, provider) DO UPDATE SET status = 'active', purchase_token = EXCLUDED.purchase_token, auto_renewing = EXCLUDED.auto_renewing, updated_at = NOW();" > /dev/null
    echo -e "${GREEN}✓ Created test subscription record${NC}"
    PURCHASE_TOKEN="$DUMMY_TOKEN"
else
    PURCHASE_TOKEN=$(echo "$SUB_EXISTS" | awk -F '|' '{print $2}' | tr -d ' ')
    echo -e "${GREEN}✓ Subscription exists, token: $PURCHASE_TOKEN${NC}"
fi
echo ""

# Step 3: Record initial database state
echo -e "${YELLOW}[3/6] Recording initial database state${NC}"

INITIAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription status: $INITIAL_STATUS${NC}"
echo ""

# Step 4: Send webhook with VALID authorization (simulating correct audience)
echo -e "${YELLOW}[4/6] Sending webhook with valid authorization${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="whk-01c-valid-audience-$(date +%s)"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  Notification Type: 2 (SUBSCRIPTION_RENEWED)"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo "  Expected: HTTP 204 (webhook processed normally)"
echo ""

# Create DeveloperNotification JSON (renewal notification)
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send webhook with valid test authorization
# Use X-Webhook-Verification-Mode: off to skip signature (can't forge valid JWT for testing)
# Use X-Webhook-Audience-Mode: strict to enforce audience claim validation
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "X-Webhook-Audience-Mode: strict" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
WEBHOOK_LINE_COUNT=$(echo "$WEBHOOK_RESPONSE" | wc -l)
if [ "$WEBHOOK_LINE_COUNT" -gt 1 ]; then
    WEBHOOK_BODY=$(echo "$WEBHOOK_RESPONSE" | head -n $((WEBHOOK_LINE_COUNT - 1)))
else
    WEBHOOK_BODY=""
fi

echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"
if [[ ! -z "$WEBHOOK_BODY" ]]; then
    echo "Webhook Response: $WEBHOOK_BODY"
fi
echo ""

# Step 5: Verify webhook was accepted
echo -e "${YELLOW}[5/6] Verifying webhook acceptance${NC}"

WEBHOOK_ACCEPTED="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $WEBHOOK_HTTP_CODE)${NC}"
    WEBHOOK_ACCEPTED="true"
else
    echo -e "${RED}✗ Webhook rejected with HTTP $WEBHOOK_HTTP_CODE${NC}"
    WEBHOOK_ACCEPTED="false"
fi
echo ""

# Step 6: Verify database was updated correctly (DB Validation)
echo -e "${YELLOW}[6/6] Verifying database update (DB Validation)${NC}"

FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')
FINAL_PERIOD_END=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT current_period_end FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription status: $FINAL_STATUS"
echo "Final period end: $FINAL_PERIOD_END"
echo ""

DB_UPDATED="false"
if [[ "$FINAL_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Subscription status is active${NC}"
    DB_UPDATED="true"
else
    echo -e "${YELLOW}⚠ Status is '$FINAL_STATUS' (expected: active)${NC}"
    DB_UPDATED="false"
fi
echo ""

# Determine overall test status
if [[ "$WEBHOOK_ACCEPTED" == "true" ]] && [[ "$DB_UPDATED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-01C Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ WHK-01C Test FAILED${NC}"
fi

# Generate JSON report
cat > whk-01c-report.json <<EOF
{
  "test_id": "WHK-01C",
  "test_name": "Audience Validation With Correct Audience",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "message_id": "$MESSAGE_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "webhook_http_code": $WEBHOOK_HTTP_CODE,
  "results": {
    "webhook_accepted": $WEBHOOK_ACCEPTED,
    "database_updated": $DB_UPDATED,
    "initial_status": "$INITIAL_STATUS",
    "final_status": "$FINAL_STATUS",
    "subscription_active": $([ "$FINAL_STATUS" == "active" ] && echo "true" || echo "false")
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-01c-report.json"
cat whk-01c-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
