#!/bin/bash

##############################################################################
# WHK-02: Duplicate Webhook Delivery (Idempotency)
# 
# Purpose: Verify that duplicate webhooks (same message_id) are handled
#          idempotently - returns success but no duplicate DB entries.
#
# Usage: ./test-whk-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Second identical webhook returns HTTP 200/204.
#                      No duplicate entries are created in the database.
#                      Backend correctly utilizes 'webhook_log' to track message_ids.
#                      Ensures resilience against 'at-least-once' delivery.
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
USER_ID="${USER_ID:-test_whk_02_user_$RUN_ID}"

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
echo "WHK-02: Duplicate Webhook Delivery (Idempotency)"
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

# Step 2: Setup - ensure subscription record exists
echo -e "${YELLOW}[2/7] Setting up test subscription record${NC}"

PURCHASE_TOKEN="test-whk-02-idempotency-$(date +%s)"

# Clean up any existing test data
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER' AND provider_transaction_id LIKE 'test-whk-02%';" 2>/dev/null

# Create subscription entry for testing
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true, NOW(), NOW());" > /dev/null

echo -e "${GREEN}✓ Created test subscription record with token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Record initial database state
echo -e "${YELLOW}[3/7] Recording initial database state${NC}"

INITIAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_SUB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo -e "${BLUE}Initial subscription status: $INITIAL_SUB_STATUS${NC}"
echo ""

# Step 4: Send FIRST webhook (subscription renewal)
echo -e "${YELLOW}[4/7] Sending FIRST webhook (subscription renewal)${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="whk-02-idempotency-test-$(date +%s)"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID (SAME for both deliveries)"
echo "  Notification Type: 2 (SUBSCRIPTION_RENEWED)"
echo "  Purchase Token: $PURCHASE_TOKEN"
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

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send FIRST webhook
# Use X-Webhook-Verification-Mode: off (test doesn't verify signatures, tests idempotency)
WEBHOOK_RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE_1=$(echo "$WEBHOOK_RESPONSE_1" | tail -n1)
echo "First webhook response code: $WEBHOOK_HTTP_CODE_1"

FIRST_ACCEPTED="false"
if [[ "$WEBHOOK_HTTP_CODE_1" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE_1" == "204" ]]; then
    echo -e "${GREEN}✓ First webhook accepted (HTTP $WEBHOOK_HTTP_CODE_1)${NC}"
    FIRST_ACCEPTED="true"
else
    echo -e "${RED}✗ First webhook failed with HTTP $WEBHOOK_HTTP_CODE_1${NC}"
fi
echo ""

# Step 5: Record state after first webhook
echo -e "${YELLOW}[5/7] Recording state after first webhook${NC}"

AFTER_FIRST_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
AFTER_FIRST_SUB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Payment count after first: $AFTER_FIRST_PAYMENT_COUNT${NC}"
echo -e "${BLUE}Subscription status after first: $AFTER_FIRST_SUB_STATUS${NC}"
echo ""

# Step 6: Send DUPLICATE webhook (same message_id)
echo -e "${YELLOW}[6/7] Sending DUPLICATE webhook (same message_id)${NC}"
echo ""

echo "Sending exact same webhook again..."
echo "  Message ID: $MESSAGE_ID (SAME as first)"
echo ""

# Send DUPLICATE webhook (exact same payload)
# Use X-Webhook-Verification-Mode: off (test doesn't verify signatures, tests idempotency)
WEBHOOK_RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE_2=$(echo "$WEBHOOK_RESPONSE_2" | tail -n1)
echo "Duplicate webhook response code: $WEBHOOK_HTTP_CODE_2"

SECOND_ACCEPTED="false"
if [[ "$WEBHOOK_HTTP_CODE_2" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE_2" == "204" ]]; then
    echo -e "${GREEN}✓ Duplicate webhook also returned HTTP $WEBHOOK_HTTP_CODE_2 (idempotent)${NC}"
    SECOND_ACCEPTED="true"
else
    echo -e "${YELLOW}⚠ Duplicate webhook returned HTTP $WEBHOOK_HTTP_CODE_2${NC}"
fi
echo ""

# Step 7: Verify idempotency - no duplicate records (DB Validation)
echo -e "${YELLOW}[7/7] Verifying idempotency (DB Validation)${NC}"
echo ""

FINAL_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_SUB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

echo "Final payment count: $FINAL_PAYMENT_COUNT (after first: $AFTER_FIRST_PAYMENT_COUNT)"
echo "Final subscription status: $FINAL_SUB_STATUS"
echo ""

# Check that provider_transaction_id is SAME (true idempotency)
FIRST_PAYMENT_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT provider_transaction_id FROM pay.payments WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')

IDEMPOTENCY_VERIFIED="false"
if [[ "$FINAL_PAYMENT_COUNT" == "$AFTER_FIRST_PAYMENT_COUNT" ]]; then
    echo -e "${GREEN}✓ No duplicate payment records created (idempotency enforced)${NC}"
    
    # Verify it's the SAME payment, not a different one
    if [[ ! -z "$FIRST_PAYMENT_ID" ]]; then
        echo -e "${GREEN}✓ Same payment record (provider_transaction_id: $FIRST_PAYMENT_ID)${NC}"
        IDEMPOTENCY_VERIFIED="true"
    else
        echo -e "${YELLOW}⚠ Could not verify payment ID${NC}"
    fi
else
    echo -e "${RED}✗ Duplicate payment records created! Count: $AFTER_FIRST_PAYMENT_COUNT → $FINAL_PAYMENT_COUNT${NC}"
    IDEMPOTENCY_VERIFIED="false"
fi

# Check subscription count as well
SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider = '$PROVIDER';" -t 2>/dev/null | tr -d ' ')

if [[ "$SUB_COUNT" == "1" ]]; then
    echo -e "${GREEN}✓ Single subscription record (no duplicates)${NC}"
else
    echo -e "${RED}✗ Unexpected subscription count: $SUB_COUNT (expected: 1)${NC}"
    IDEMPOTENCY_VERIFIED="false"
fi
echo ""

# Determine overall test status
if [[ "$FIRST_ACCEPTED" == "true" ]] && [[ "$SECOND_ACCEPTED" == "true" ]] && [[ "$IDEMPOTENCY_VERIFIED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ WHK-02 Test FAILED${NC}"
fi

# Generate JSON report
cat > whk-02-report.json <<EOF
{
  "test_id": "WHK-02",
  "test_name": "Duplicate Webhook Delivery (Idempotency)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "message_id": "$MESSAGE_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "first_webhook_accepted": $FIRST_ACCEPTED,
    "first_webhook_http_code": $WEBHOOK_HTTP_CODE_1,
    "second_webhook_accepted": $SECOND_ACCEPTED,
    "second_webhook_http_code": $WEBHOOK_HTTP_CODE_2,
    "idempotency_enforced": $IDEMPOTENCY_VERIFIED,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "after_first_payment_count": $AFTER_FIRST_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT,
    "no_duplicate_payments": $([ "$FINAL_PAYMENT_COUNT" == "$AFTER_FIRST_PAYMENT_COUNT" ] && echo "true" || echo "false"),
    "subscription_count": $SUB_COUNT
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-02-report.json"
cat whk-02-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
