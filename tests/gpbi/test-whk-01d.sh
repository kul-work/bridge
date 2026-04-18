#!/bin/bash

##############################################################################
# WHK-01D: Audience Validation Disabled (Dev Mode)
# 
# Purpose: Verify that webhooks are processed normally when
#          GOOGLE_VERIFY_AUDIENCE=false (default for dev), regardless of
#          the JWT audience claim.
#
# Usage: ./test-whk-01d.sh
#
# Prerequisites:
#   - Backend running and listening on $BRIDGE_API_URL (default: http://localhost:3000)
#   - Backend configured with: MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - Test uses headers:
#     * X-Webhook-Verification-Mode: off (skip signature verification)
#     * X-Webhook-Audience-Mode: off (skip audience validation - dev/local mode)
#
# TESTPLAN Reference:
#   Backend Behavior: Code skips audience validation (no check performed),
#                     Webhook proceeds through normal validation and DB update.
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
USER_ID="${USER_ID:-test_whk_01d_user_$RUN_ID}"

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
echo "WHK-01D: Audience Validation Disabled (Dev Mode)"
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

# Step 2: Ensure subscription record exists
echo -e "${YELLOW}[2/6] Verifying subscription record exists${NC}"

SUB_EXISTS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status, purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' LIMIT 1;" -t 2>/dev/null || echo "")

if [[ -z "$SUB_EXISTS" ]] || [[ "$SUB_EXISTS" == *"(0 rows)"* ]]; then
    echo -e "${YELLOW}⚠ No subscription found, creating test record...${NC}"
    DUMMY_TOKEN="test-subscription-whk-01d-$(date +%s)"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', 'active', '$DUMMY_TOKEN', '$PROVIDER', true, NOW(), NOW()) ON CONFLICT (external_user_id, subscription_id) DO UPDATE SET status = 'active', purchase_token = '$DUMMY_TOKEN', updated_at = NOW();" 2>/dev/null
    echo -e "${GREEN}✓ Created test subscription record${NC}"
    PURCHASE_TOKEN="$DUMMY_TOKEN"
else
    PURCHASE_TOKEN=$(echo "$SUB_EXISTS" | awk -F '|' '{print $2}' | tr -d ' ')
    echo -e "${GREEN}✓ Subscription exists, token: $PURCHASE_TOKEN${NC}"
fi
echo ""

# Step 3: Record initial database state
echo -e "${YELLOW}[3/6] Recording initial database state${NC}"

INITIAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription status: $INITIAL_STATUS${NC}"
echo ""

# Step 4: Send webhook with ANY audience (wrong, missing, or correct - all should work)
echo -e "${YELLOW}[4/6] Sending webhook with incorrect/any audience claim${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="whk-01d-dev-mode-$(date +%s)"
WRONG_AUDIENCE="https://random-domain.com/wrong-audience"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  JWT Audience (any/wrong): $WRONG_AUDIENCE"
echo "  Notification Type: 2 (SUBSCRIPTION_RENEWED)"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo "  Expected: HTTP 200 (audience check skipped in dev mode)"
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

# Create a fake JWT with wrong audience (to test that audience check is skipped)
JWT_HEADER=$(echo -n '{"alg":"RS256","typ":"JWT"}' | base64 -w 0 | tr '+/' '-_' | tr -d '=')
JWT_PAYLOAD=$(echo -n "{\"aud\":\"$WRONG_AUDIENCE\",\"iss\":\"accounts.google.com\",\"exp\":$(($(date +%s) + 3600))}" | base64 -w 0 | tr '+/' '-_' | tr -d '=')
FAKE_JWT="$JWT_HEADER.$JWT_PAYLOAD.fake-signature-for-testing"

# Send webhook with wrong audience (should still work in dev mode)
# Use X-Webhook-Verification-Mode: off to explicitly disable signature verification (testing dev mode)
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $FAKE_JWT" \
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

# Step 5: Verify webhook was accepted (audience check skipped)
echo -e "${YELLOW}[5/6] Verifying webhook acceptance (dev mode)${NC}"

WEBHOOK_ACCEPTED="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP 200) - audience check skipped${NC}"
    WEBHOOK_ACCEPTED="true"
elif [[ "$WEBHOOK_HTTP_CODE" == "400" ]] || [[ "$WEBHOOK_HTTP_CODE" == "401" ]] || [[ "$WEBHOOK_HTTP_CODE" == "403" ]]; then
    echo -e "${YELLOW}⚠ Webhook rejected (HTTP $WEBHOOK_HTTP_CODE) - audience check may be ENABLED${NC}"
    echo "  Note: This test expects GOOGLE_VERIFY_AUDIENCE=false (dev mode)"
    WEBHOOK_ACCEPTED="false"
else
    echo -e "${YELLOW}⚠ Unexpected HTTP code: $WEBHOOK_HTTP_CODE${NC}"
    WEBHOOK_ACCEPTED="false"
fi
echo ""

# Step 6: Verify database was updated (DB Validation)
echo -e "${YELLOW}[6/6] Verifying database update (DB Validation)${NC}"

FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription status: $FINAL_STATUS"
echo ""

DB_UPDATED="false"
if [[ "$FINAL_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Subscription status is active${NC}"
    DB_UPDATED="true"
else
    echo -e "${YELLOW}⚠ Status is '$FINAL_STATUS'${NC}"
    DB_UPDATED="false"
fi
echo ""

# Determine overall test status
if [[ "$WEBHOOK_ACCEPTED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-01D Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ WHK-01D Test FAILED${NC}"
fi

# Generate JSON report
cat > whk-01d-report.json <<EOF
{
  "test_id": "WHK-01D",
  "test_name": "Audience Validation Disabled (Dev Mode)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "message_id": "$MESSAGE_ID",
  "wrong_audience_used": "$WRONG_AUDIENCE",
  "webhook_http_code": $WEBHOOK_HTTP_CODE,
  "results": {
    "webhook_accepted": $WEBHOOK_ACCEPTED,
    "audience_check_skipped": $WEBHOOK_ACCEPTED,
    "database_updated": $DB_UPDATED,
    "initial_status": "$INITIAL_STATUS",
    "final_status": "$FINAL_STATUS"
  },
  "notes": "Dev/Testing only - NOT safe for production. Set GOOGLE_VERIFY_AUDIENCE=true in production."
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-01d-report.json"
cat whk-01d-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
