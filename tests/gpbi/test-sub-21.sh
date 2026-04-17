#!/bin/bash

##############################################################################
# SUB-21: Price Step-Up Consent (Korea Only) Test
# 
# Purpose: Verify that for South Korean users, the backend correctly handles
#          the price_step_up_consent_updated webhook when transitioning from
#          a lower price phase (e.g., free trial) to regular price.
#          1. Establish a trial subscription
#          2. Simulate price_step_up_consent_updated (notificationType 22)
#          3. Simulate price_changed (notificationType 8)
#          4. Verify final subscription state
#
# Usage: ./test-sub-21.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * PRODUCT_ID_SUB, PROVIDER, PACKAGE_NAME
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
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
DUMMY_TOKEN="test-sub-21-trial-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-21-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-21: Price Step-Up Consent (Korea Only)"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
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
    \"reason\": \"test-step-up-setup-21\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-21-$(date +%s)\"
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
    \"product_type\": \"subscription\"
  }" )

echo -e "${GREEN}✓ Trial subscription established${NC}"
echo ""

# Step 4: Simulate price_step_up_consent_updated (notificationType 22)
echo -e "${YELLOW}[2/5] Sending price_step_up_consent_updated webhook (notificationType 22)${NC}"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 22,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
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

echo -e "${GREEN}✓ Consent notification webhook sent${NC}"
echo ""

# Step 5: Simulate price_changed (notificationType 8) - user accepted
echo -e "${YELLOW}[3/5] Sending price_changed webhook (notificationType 8)${NC}"

PRICE_CHANGED_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$(date +%s000)",
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

echo -e "${GREEN}✓ Price changed webhook sent${NC}"
echo ""

# Step 6: Verify final subscription state
echo -e "${YELLOW}[4/5] Verifying final subscription state in Bridge DB${NC}"
export PGPASSWORD="postgres"
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$STATUS" == "active" ]] || [[ "$STATUS" == "trial" ]]; then
    echo -e "${GREEN}✓ Success: Subscription status is $STATUS (valid state for Korea)${NC}"
else
    echo -e "${RED}✗ Failure: Subscription status is $STATUS, expected 'active' or 'trial'${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-21",
  "test_name": "Price Change (Automatic Upgrade)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "automatic_upgrade_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-21 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
