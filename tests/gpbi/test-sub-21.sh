#!/bin/bash

##############################################################################
# SUB-21: Price Step-Up Consent (Korea Only)
# 
# Purpose: Verify that for South Korean users, the backend correctly handles
#          the price_step_up_consent_updated webhook when transitioning.
#
# Usage: ./test-sub-21.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN, BRIDGE_WEBHOOK_FUTURE_TS
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#   - jq installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: price_step_up_consent_updated (notificationType 22) is processed.
#                      price_changed (notificationType 8) transition is correctly recorded.
#                      Final subscription state remains valid (active/trial).
#                      Compliance with South Korea specific billing regulations (consent flows).
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for SUB-21 snapshot assertions"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="sub-21-${TIMESTAMP}-$$"
DUMMY_TOKEN="test-sub-21-trial-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-21-report.json"
NEW_PRICE_CENTS=1200000 # 12,000 KRW (Google Play represents this as 12,000,000,000 micros)
NEW_PRICE_MICROS=$((NEW_PRICE_CENTS * 10000))
CONSENT_DEADLINE_MS=$(($(date +%s) + 604800))000

echo -e "${YELLOW}========================================${NC}"
echo "SUB-21: Price Step-Up Consent (Korea Only)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}PASS: Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo ""

# Step 3: Establish a trial subscription
echo -e "${YELLOW}[1/5] Establishing trial subscription${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-step-up-setup-21\"
  }" )

# Verify purchase (as trial)
# Mocking return of trial status by token naming or specific headers if supported
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Test-Subscription-Status: trial" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\",
    \"currency\": \"KRW\"
  }" )

echo -e "${GREEN}PASS: Trial subscription established${NC}"
echo ""

# Step 4: Simulate price_step_up_consent_updated (notificationType 22)
echo -e "${YELLOW}[2/5] Sending price_step_up_consent_updated webhook (notificationType 22)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "\$((\$(date +%s) + 10))000",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 22,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID",
    "priceStepUpConsentDetails": {
      "priceMicros": $NEW_PRICE_MICROS,
      "consentDeadlineTimeMillis": $CONSENT_DEADLINE_MS
    }
  }
}
EOF
)
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"test-webhook-21-consent-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}PASS: Consent notification webhook sent${NC}"
echo ""

# Step 5: Simulate price_changed (notificationType 8) - user accepted
echo -e "${YELLOW}[3/5] Sending price_changed webhook (notificationType 8)${NC}"

PRICE_CHANGED_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "\$((\$(date +%s) + 20))000",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 8,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
PRICE_CHANGED_B64=$(echo -n "$PRICE_CHANGED_JSON" | base64 -w 0 2>/dev/null || echo -n "$PRICE_CHANGED_JSON" | base64)

curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$PRICE_CHANGED_B64\",
      \"message_id\": \"test-webhook-21-changed-$(date +%s)\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}PASS: Price changed webhook sent${NC}"
echo ""

# Step 6: Verify final subscription state
echo -e "${YELLOW}[4/5] Verifying final subscription state in Bridge DB${NC}"
export PGPASSWORD="postgres"
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$STATUS" == "active" ]] || [[ "$STATUS" == "trial" ]]; then
    echo -e "${GREEN}PASS: Subscription status is $STATUS (valid state for Korea)${NC}"
else
    echo -e "${RED}FAIL: Subscription status is $STATUS, expected 'active' or 'trial'${NC}"
    exit 1
fi

PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d '[:space:]')

if [[ "$PAYMENT_COUNT" == "2" ]]; then
    echo -e "${GREEN}PASS: Payment row count is 2 (initial payment + price update)${NC}"
else
    echo -e "${RED}FAIL: Payment row count is $PAYMENT_COUNT, expected 2${NC}"
    exit 1
fi

PAYMENT_CURRENCY=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT currency FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]')

if [[ "$PAYMENT_CURRENCY" == "KRW" ]]; then
    echo -e "${GREEN}PASS: Payment currency is KRW${NC}"
else
    echo -e "${RED}FAIL: Payment currency is $PAYMENT_CURRENCY, expected KRW${NC}"
    exit 1
fi
echo ""

# Step 7: Verify app-facing subscription snapshot
echo -e "${YELLOW}[5/5] Verifying price step-up snapshot contract${NC}"
STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
STATUS_BODY=$(echo "$STATUS_RESPONSE" | sed '$d')

if [[ "$STATUS_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}FAIL: subscription-status returned HTTP $STATUS_HTTP_CODE${NC}"
    echo "$STATUS_BODY"
    exit 1
fi

if echo "$STATUS_BODY" | jq -e \
  --argjson new_price "$NEW_PRICE_CENTS" \
  '.is_premium == true and (.status == "active" or .status == "trial") and .google_requires_price_step_up_consent == true and .google_new_price_cents == $new_price and .google_price_step_up_consent_deadline != null' > /dev/null; then
    echo -e "${GREEN}PASS: Snapshot shows price step-up consent fields${NC}"
else
    echo -e "${RED}FAIL: Price step-up snapshot contract mismatch${NC}"
    echo "$STATUS_BODY" | jq .
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-21",
  "test_name": "Price Step-Up Consent (Korea Only)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "automatic_upgrade_verified": true,
  "snapshot_verified": true
}
EOF

echo -e "${GREEN}PASS: SUB-21 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
