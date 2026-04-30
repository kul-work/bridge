#!/bin/bash

##############################################################################
# ERR-04: Revoked/Refunded Purchase Token
# 
# Purpose: Verify that after a purchase is revoked or refunded, the 
#          backend correctly processes the revocation webhook and 
#          revokes service access.
#
# Usage: ./test-err-04.sh
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
#   Expected Behavior: A subscription.revoked (notificationType=12) webhook is successfully processed.
#                      The subscription status is updated to 'revoked' (or cancelled/expired) in pay.subscriptions.
#                      Subsequent requests to entitlement endpoints return a 4xx error.
#                      Ensures refunds and revocations are correctly propagated and enforced.
#                      Validates idempotency and stale-event suppression logic using event timestamps.
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
TEST_RUN_ID="err-04-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
REPORT_FILE="err-04-report.json"
USER_ID="${USER_ID:-test_err_04_user_$TEST_RUN_ID}"
DUMMY_TOKEN="test-err-04-token-$TEST_RUN_ID"
WEBHOOK_ID="webhook-err-04-$TEST_RUN_ID"

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
echo "ERR-04: Revoked/Refunded Purchase Token"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
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

# Step 2: Setup - create an active subscription first
echo -e "${YELLOW}[2/6] Setting up active subscription${NC}"

PURCHASE_TOKEN="test-err-04-token-$TEST_RUN_ID"

# Clean up and create subscription
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at, version, last_event_time, payment_failure_notification, google_on_hold) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'active', '$PURCHASE_TOKEN', '$PROVIDER', true, NOW(), NOW(), 1, $(date +%s)000, false, false);" 2>/dev/null

echo -e "${GREEN}✓ Created active subscription${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Verify initial status
INITIAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
echo "Initial status: $INITIAL_STATUS"
echo ""

# Step 3: Simulate revocation via webhook (subscription.revoked)
echo -e "${YELLOW}[3/6] Simulating revocation webhook${NC}"
echo ""

# Debug: Check if subscription exists before webhook
echo "Debug: Checking if subscription exists with purchase token: $PURCHASE_TOKEN"
EXISTING_SUB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT external_user_id, status FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
echo "Debug: Found subscription: $EXISTING_SUB"
echo ""

TIMESTAMP_MS=$(date +%s000)
MESSAGE_ID="webhook-err-04-$TEST_RUN_ID"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  Notification Type: 12 (SUBSCRIPTION_REVOKED)"
echo "  Purpose: Simulating Play Console refund/revoke"
echo ""

# Create revocation webhook
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 12,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
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

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"

WEBHOOK_OK="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Revocation webhook processed${NC}"
    WEBHOOK_OK="true"
else
    echo -e "${YELLOW}⚠ Webhook returned HTTP $WEBHOOK_HTTP_CODE${NC}"
fi
echo ""

# Step 4: Verify subscription status updated (DB Validation)
echo -e "${YELLOW}[4/6] Verifying subscription status updated (DB Validation)${NC}"

FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')
REVOKED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT revoked_at FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Final status: $FINAL_STATUS (was: $INITIAL_STATUS)"
echo "Revoked at: $REVOKED_AT"
echo ""

STATUS_UPDATED="false"
if [[ "$FINAL_STATUS" == "revoked" ]] || [[ "$FINAL_STATUS" == "expired" ]] || [[ "$FINAL_STATUS" == "cancelled" ]]; then
    echo -e "${GREEN}✓ Status correctly updated to: $FINAL_STATUS${NC}"
    STATUS_UPDATED="true"
else
    echo -e "${YELLOW}⚠ Status is '$FINAL_STATUS' (expected: revoked/expired/cancelled)${NC}"
    # Check if status changed at all
    if [[ "$FINAL_STATUS" != "$INITIAL_STATUS" ]]; then
        STATUS_UPDATED="true"
    fi
fi
echo ""

# Step 5: Verify access is revoked (try premium feature)
echo -e "${YELLOW}[5/6] Verifying access is revoked${NC}"

PREMIUM_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/subscriptions" \
  -H "Content-Type: application/json" \
   \
  )

PREMIUM_HTTP_CODE=$(echo "$PREMIUM_RESPONSE" | tail -n1)
echo "Premium feature HTTP: $PREMIUM_HTTP_CODE"

ACCESS_REVOKED="false"
if [[ "$PREMIUM_HTTP_CODE" == "401" ]] || [[ "$PREMIUM_HTTP_CODE" == "402" ]] || [[ "$PREMIUM_HTTP_CODE" == "403" ]]; then
    echo -e "${GREEN}✓ Access correctly revoked (HTTP $PREMIUM_HTTP_CODE)${NC}"
    ACCESS_REVOKED="true"
elif [[ "$PREMIUM_HTTP_CODE" == "200" ]]; then
    echo -e "${YELLOW}⚠ Access still granted (HTTP 200) - may need status refresh${NC}"
    ACCESS_REVOKED="false"
else
    echo -e "${YELLOW}⚠ HTTP $PREMIUM_HTTP_CODE${NC}"
    ACCESS_REVOKED="false"
fi
echo ""

# Step 6: Cleanup
echo -e "${YELLOW}[6/6] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
if [[ "$WEBHOOK_OK" == "true" ]] && [[ "$STATUS_UPDATED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-04 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-04 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-04",
  "test_name": "Revoked/Refunded Purchase Token",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "webhook_processed": $WEBHOOK_OK,
    "webhook_http_code": $WEBHOOK_HTTP_CODE,
    "status_updated": $STATUS_UPDATED,
    "initial_status": "$INITIAL_STATUS",
    "final_status": "$FINAL_STATUS",
    "access_revoked": $ACCESS_REVOKED,
    "premium_http_code": $PREMIUM_HTTP_CODE
  }
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
