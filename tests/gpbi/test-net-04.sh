#!/bin/bash

##############################################################################
# NET-04: Webhook Arrives While verify_payment In-Flight
# 
# Purpose: Verify that when webhook and verify_payment are processed
#          concurrently (race condition), the final state is correct
#          with no duplication or data loss.
#
# Usage: ./test-net-04.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: One request succeeds, the other may conflict or be
#                      idempotent. Final state: Subscription is Active,
#                      no duplication or data loss.
#   Backend State: Both requests call get_subscription() concurrently.
#                  DB update uses subscription_id + user_id as unique key.
#                  Idempotency key prevents double-state-change.
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
    echo "Usage: ./test-net-04.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "NET-04: Webhook Arrives While verify_payment In-Flight"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id from email
echo -e "${YELLOW}[1/6] Fetching user_id from database for email: $EMAIL${NC}"

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

# Step 2: Clean up any existing subscription data
echo -e "${YELLOW}[2/6] Cleaning up previous test data${NC}"

PURCHASE_TOKEN="test-net-04-concurrent-$(date +%s)"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

echo -e "${GREEN}✓ Cleanup complete${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Prepare webhook payload
echo -e "${YELLOW}[3/6] Preparing concurrent requests${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="net-04-concurrent-$(date +%s)"

# Create DeveloperNotification JSON
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 4,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

echo "Concurrent request details:"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo "  Webhook Message ID: $MESSAGE_ID"
echo ""

# Step 4: Send both requests CONCURRENTLY (simulating race condition)
echo -e "${YELLOW}[4/6] Sending verify_payment AND webhook CONCURRENTLY${NC}"
echo ""

# Create temp files for responses
VERIFY_TEMP=$(mktemp)
WEBHOOK_TEMP=$(mktemp)

# Launch both requests in background (concurrent execution)
echo "Launching concurrent requests..."

# verify_payment request (background)
(curl -s -w "\n%{http_code}" -X POST \
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
  }" > "$VERIFY_TEMP" 2>&1) &
VERIFY_PID=$!

# Webhook request (background)
(curl -s -w "\n%{http_code}" -X POST \
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
  }" > "$WEBHOOK_TEMP" 2>&1) &
WEBHOOK_PID=$!

# Wait for both to complete
echo "Waiting for both requests to complete..."
wait $VERIFY_PID 2>/dev/null || true
wait $WEBHOOK_PID 2>/dev/null || true

# Read responses
VERIFY_RESPONSE=$(cat "$VERIFY_TEMP")
WEBHOOK_RESPONSE=$(cat "$WEBHOOK_TEMP")

# Cleanup temp files
rm -f "$VERIFY_TEMP" "$WEBHOOK_TEMP"

# Parse responses
VERIFY_HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)

echo ""
echo "Results:"
echo "  verify_payment HTTP: $VERIFY_HTTP_CODE"
echo "  webhook HTTP: $WEBHOOK_HTTP_CODE"
echo ""

VERIFY_OK="false"
if [[ "$VERIFY_HTTP_CODE" == "200" ]]; then
    echo -e "  ${GREEN}✓ verify_payment succeeded${NC}"
    VERIFY_OK="true"
else
    echo -e "  ${YELLOW}⚠ verify_payment returned $VERIFY_HTTP_CODE${NC}"
fi

WEBHOOK_OK="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]]; then
    echo -e "  ${GREEN}✓ webhook succeeded${NC}"
    WEBHOOK_OK="true"
else
    echo -e "  ${YELLOW}⚠ webhook returned $WEBHOOK_HTTP_CODE${NC}"
fi
echo ""

# Step 5: Verify final state (no duplicates, correct data)
echo -e "${YELLOW}[5/6] Verifying final state (DB Validation)${NC}"
echo ""

# Check subscription count (should be exactly 1)
SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
TOKEN_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Subscriptions for user+product: $SUB_COUNT"
echo "Subscriptions with this token: $TOKEN_COUNT"

# STRICT: Both counts must be exactly 1
NO_DUPLICATES="false"
if [[ "$SUB_COUNT" -ne "1" ]] || [[ "$TOKEN_COUNT" -ne "1" ]]; then
    echo -e "${RED}✗ Race condition NOT handled safely!"
    echo "  Expected: SUB_COUNT=1, TOKEN_COUNT=1"
    echo "  Got: SUB_COUNT=$SUB_COUNT, TOKEN_COUNT=$TOKEN_COUNT${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Exactly 1 subscription, 1 token (safe)${NC}"
    NO_DUPLICATES="true"
fi
echo ""

# Check status (MUST be "active")
FINAL_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ' || echo "")

STATUS_OK="false"
if [[ "$FINAL_STATUS" != "active" ]]; then
    echo -e "${RED}✗ Expected 'active', got '$FINAL_STATUS'${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Final status: active${NC}"
    STATUS_OK="true"
fi
echo ""

# Step 6: Cleanup
echo -e "${YELLOW}[6/6] Cleanup${NC}"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND purchase_token LIKE 'test-net-04%';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
# At least one request should succeed, and no duplicates
AT_LEAST_ONE_OK="false"
if [[ "$VERIFY_OK" == "true" ]] || [[ "$WEBHOOK_OK" == "true" ]]; then
    AT_LEAST_ONE_OK="true"
fi

if [[ "$AT_LEAST_ONE_OK" == "true" ]] && [[ "$NO_DUPLICATES" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ NET-04 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ NET-04 Test FAILED${NC}"
fi

# Generate JSON report
cat > net-04-report.json <<EOF
{
  "test_id": "NET-04",
  "test_name": "Webhook Arrives While verify_payment In-Flight",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "purchase_token": "$PURCHASE_TOKEN",
  "message_id": "$MESSAGE_ID",
  "results": {
    "verify_payment_http": $VERIFY_HTTP_CODE,
    "verify_payment_success": $VERIFY_OK,
    "webhook_http": $WEBHOOK_HTTP_CODE,
    "webhook_success": $WEBHOOK_OK,
    "at_least_one_succeeded": $AT_LEAST_ONE_OK,
    "no_duplicate_subscriptions": $NO_DUPLICATES,
    "subscription_count": $SUB_COUNT,
    "token_count": $TOKEN_COUNT,
    "final_status": "$FINAL_STATUS"
  },
  "notes": "Backend handles concurrent requests via database unique constraints and idempotency"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: net-04-report.json"
cat net-04-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
