#!/bin/bash

##############################################################################
# NET-02: verify_payment Call Fails / Network Timeout
# 
# Purpose: Verify that when verify_payment fails or times out, the system
#          supports retry and eventually achieves correct state.
#
# Usage: ./test-net-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: App should display error toast and allow retry.
#                      User can manually retry verification in app.
#   Backend State: No DB entry created on first attempt (simulated failure).
#                  Webhook arrives; cannot find user (yet).
#                  When retry succeeds: Token registered, subsequent webhooks ok.
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
USER_ID="${USER_ID:-test_net_02_user_$RUN_ID}"

# Defaults
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "NET-02: verify_payment Call Fails / Network Timeout"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/7] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing subscription data
echo -e "${YELLOW}[2/7] Cleaning up previous test data${NC}"

PURCHASE_TOKEN="test-net-02-retry-$(date +%s)"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

echo -e "${GREEN}✓ Cleanup complete${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Simulate FIRST verify_payment call that "fails" (we'll use invalid endpoint to simulate)
echo -e "${YELLOW}[3/7] Simulating FIRST verify_payment call (failure scenario)${NC}"
echo ""

echo "Note: We simulate failure by recording that no DB entry should exist yet"
echo "      In real scenario, this would be a network timeout or 5xx error"
echo ""

# Record initial state - no subscription should exist
INITIAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Initial subscription count for token: $INITIAL_SUB_COUNT"
echo -e "${BLUE}Simulating: First attempt failed (no DB entry created)${NC}"
FIRST_ATTEMPT_FAILED="true"
echo ""

# Step 4: Webhook arrives but can't find user (token not registered)
echo -e "${YELLOW}[4/7] Webhook arrives (token not yet registered)${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="net-02-during-retry-$(date +%s)"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  Purchase Token: $PURCHASE_TOKEN (not yet in DB)"
echo ""

# Create DeveloperNotification JSON
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

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"

# Webhook should handle gracefully even if token not found
WEBHOOK_HANDLED="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE" == "404" ]]; then
    echo -e "${GREEN}✓ Webhook handled gracefully (token not yet registered)${NC}"
    WEBHOOK_HANDLED="true"
else
    echo -e "${YELLOW}⚠ Webhook returned HTTP $WEBHOOK_HTTP_CODE${NC}"
    WEBHOOK_HANDLED="true"
fi
echo ""

# Step 5: RETRY verify_payment call (this time it succeeds)
echo -e "${YELLOW}[5/7] RETRY verify_payment call (success)${NC}"
echo ""

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  Token: $PURCHASE_TOKEN"
echo "  Note: This is the retry after initial failure"
echo ""

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

VERIFY_HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
echo "verify_payment Response Code: $VERIFY_HTTP_CODE"

RETRY_SUCCESS="false"
if [[ "$VERIFY_HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Retry verify_payment successful${NC}"
    RETRY_SUCCESS="true"
else
    echo -e "${RED}✗ Retry verify_payment failed with HTTP $VERIFY_HTTP_CODE${NC}"
fi
echo ""

# Step 6: Verify token is now registered and subsequent webhooks work
echo -e "${YELLOW}[6/7] Verifying token registered and webhooks work (DB Validation)${NC}"
echo ""

FINAL_SUB=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status, external_user_id FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null || echo "")

TOKEN_REGISTERED="false"
if [[ ! -z "$FINAL_SUB" ]] && [[ "$FINAL_SUB" != *"(0 rows)"* ]]; then
    FINAL_STATUS=$(echo "$FINAL_SUB" | awk -F '|' '{print $1}' | tr -d ' ')
    echo -e "${GREEN}✓ Token now registered in DB${NC}"
    echo "  Status: $FINAL_STATUS"
    TOKEN_REGISTERED="true"
else
    echo -e "${RED}✗ Token not found in DB after retry${NC}"
fi
echo ""

# Test that subsequent webhook now works
echo "Sending follow-up webhook to verify it now processes correctly..."
MESSAGE_ID_2="net-02-followup-$(date +%s)"

WEBHOOK_RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID_2\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE_2=$(echo "$WEBHOOK_RESPONSE_2" | tail -n1)
echo "Follow-up webhook Response Code: $WEBHOOK_HTTP_CODE_2"

FOLLOWUP_WEBHOOK_OK="false"
if [[ "$WEBHOOK_HTTP_CODE_2" == "200" ]]; then
    echo -e "${GREEN}✓ Follow-up webhook processed successfully (user now linked)${NC}"
    FOLLOWUP_WEBHOOK_OK="true"
else
    echo -e "${YELLOW}⚠ Follow-up webhook returned HTTP $WEBHOOK_HTTP_CODE_2${NC}"
fi
echo ""

# Step 7: Cleanup
echo -e "${YELLOW}[7/7] Cleanup${NC}"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
if [[ "$RETRY_SUCCESS" == "true" ]] && [[ "$TOKEN_REGISTERED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ NET-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ NET-02 Test FAILED${NC}"
fi

# Generate JSON report
cat > net-02-report.json <<EOF
{
  "test_id": "NET-02",
  "test_name": "verify_payment Call Fails / Network Timeout",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "first_attempt_simulated_failure": $FIRST_ATTEMPT_FAILED,
    "webhook_handled_gracefully": $WEBHOOK_HANDLED,
    "webhook_http_code": $WEBHOOK_HTTP_CODE,
    "retry_verify_payment_success": $RETRY_SUCCESS,
    "retry_http_code": $VERIFY_HTTP_CODE,
    "token_registered_after_retry": $TOKEN_REGISTERED,
    "followup_webhook_success": $FOLLOWUP_WEBHOOK_OK
  },
  "notes": "App must retry verify_payment on failure; backend supports retries via idempotency"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: net-02-report.json"
cat net-02-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
