#!/bin/bash

##############################################################################
# SUB-22: Out-of-App Resubscribe Linking (SUB-RESUB-01)
# 
# Purpose: Verify out-of-app purchase context linking when user resubscribes
#          after subscription expiry.
#
# Usage: ./test-sub-22.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: New purchase_token is correctly linked to the existing external_user_id in pay.subscriptions.
#                      Status returns to 'active'.
#                      Backend correctly identifies the user identity via provider history.
#                      Ensures users who resubscribe outside the app retain their historical identity.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="sub-22-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
OLD_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-22-old-$TEST_RUN_ID"
NEW_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-22-new-$TEST_RUN_ID"
REPORT_FILE="sub-22-report.json"
OLD_REGISTER_HTTP_CODE=0
OLD_VERIFY_HTTP_CODE=0
EXPIRATION_WEBHOOK_HTTP_CODE=0
NEW_REGISTER_HTTP_CODE=0
NEW_VERIFY_HTTP_CODE=0

echo -e "${YELLOW}========================================${NC}"
echo "SUB-22: Out-of-App Resubscribe Linking"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"

fail_test() {
    local failure_step="$1"
    local details="$2"
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-22",
  "test_name": "Out-of-App Resubscribe Linking",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "failure_step": "$failure_step",
  "details": "$details",
  "old_register_http_code": $OLD_REGISTER_HTTP_CODE,
  "old_verify_http_code": $OLD_VERIFY_HTTP_CODE,
  "expiration_webhook_http_code": $EXPIRATION_WEBHOOK_HTTP_CODE,
  "new_register_http_code": $NEW_REGISTER_HTTP_CODE,
  "new_verify_http_code": $NEW_VERIFY_HTTP_CODE
}
EOF
    echo -e "${RED}SUB-22 failed at $failure_step: $details${NC}"
    echo "Report saved to: $REPORT_FILE"
    exit 1
}

echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo ""

# Step 3: Establish initial subscription
echo -e "${YELLOW}[1/5] Establishing initial subscription (OLD Token)${NC}"

# Pre-register
OLD_REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-oap-old-22\"
  }" )

if [[ "$OLD_REGISTER_HTTP_CODE" != "200" ]]; then
    fail_test "old_register" "expected HTTP 200, got $OLD_REGISTER_HTTP_CODE"
fi

# Verify
OLD_VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$OLD_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

if [[ "$OLD_VERIFY_HTTP_CODE" != "200" ]]; then
    fail_test "old_verify" "expected HTTP 200, got $OLD_VERIFY_HTTP_CODE"
fi

echo -e "${GREEN}✓ Initial subscription established${NC}"
echo ""

# Step 4: Simulate Expiration
echo -e "${YELLOW}[2/5] Simulating Expiration (notificationType 13)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 13,
    "purchaseToken": "$OLD_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

EXPIRATION_WEBHOOK_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"test-webhook-22-exp-$(date +%s)\",
      \"attributes\": {}
    }
  }")

if [[ "$EXPIRATION_WEBHOOK_HTTP_CODE" != "200" && "$EXPIRATION_WEBHOOK_HTTP_CODE" != "204" ]]; then
    fail_test "expiration_webhook" "expected HTTP 200 or 204, got $EXPIRATION_WEBHOOK_HTTP_CODE"
fi

echo -e "${GREEN}✓ Expiration webhook sent${NC}"
echo ""

# Step 5: Resubscribe with NEW Token (Out-of-App)
echo -e "${YELLOW}[3/5] Resubscribing with NEW Token (Out-of-App)${NC}"

# Pre-register for the NEW purchase (simulating app noticing new subscription)
# In Out-of-App, identifying the account usually relies on the backend finding the previous owner.
NEW_REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-oap-new-22\"
  }" )

if [[ "$NEW_REGISTER_HTTP_CODE" != "200" ]]; then
    fail_test "new_register" "expected HTTP 200, got $NEW_REGISTER_HTTP_CODE"
fi

# Verify NEW token
# The backend should link this token to USER_ID because it belongs to the same Google account.
NEW_VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$NEW_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

if [[ "$NEW_VERIFY_HTTP_CODE" != "200" ]]; then
    fail_test "new_verify" "expected HTTP 200, got $NEW_VERIFY_HTTP_CODE"
fi

echo -e "${GREEN}✓ Resubscription verification requested${NC}"
echo ""

# Step 6: Verify Linking in DB
echo -e "${YELLOW}[4/5] Verifying resubscription linking in Bridge DB${NC}"
export PGPASSWORD="postgres"
bridge_wait_for_db_glob \
    RES_DATA \
    "SELECT external_user_id, status FROM pay.subscriptions WHERE purchase_token = '$NEW_TOKEN';" \
    "*$USER_ID*active*" \
    10 \
    1 || true

if [[ "$RES_DATA" == *"$USER_ID"*"active"* ]]; then
    echo -e "${GREEN}✓ Success: New token $NEW_TOKEN correctly linked to $USER_ID with status 'active'${NC}"
else
    fail_test "resubscription_link" "expected new token linked to $USER_ID with active status, got $RES_DATA"
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-22",
  "test_name": "Out-of-App Resubscribe Linking",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "old_register_http_code": $OLD_REGISTER_HTTP_CODE,
  "old_verify_http_code": $OLD_VERIFY_HTTP_CODE,
  "expiration_webhook_http_code": $EXPIRATION_WEBHOOK_HTTP_CODE,
  "new_register_http_code": $NEW_REGISTER_HTTP_CODE,
  "new_verify_http_code": $NEW_VERIFY_HTTP_CODE,
  "resubscribe_link_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-22 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
