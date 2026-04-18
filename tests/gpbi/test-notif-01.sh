#!/bin/bash

##############################################################################
# NOTIF-01: Payment Failure & Acknowledgment
#
# Purpose: Verify payment failure sets notification flag, and user can acknowledge it.
#
# Usage: ./test-notif-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured
#
# Test Flow:
#   1. Initial purchase (Active)
#   2. Trigger Account Hold (simulates payment failure)
#   3. Verify payment_failure_notification=true via API
#   4. Call acknowledge endpoint
#   5. Verify payment_failure_notification=false via API
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Config
RUN_ID="$(date +%s)-$RANDOM"
DUMMY_TOKEN="test-token-notif01-$RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
USER_ID="${USER_ID:-test_notif_01_user_$RUN_ID}"

# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "NOTIF-01: Payment Failure & Acknowledgment"
echo -e "${YELLOW}========================================${NC}"
echo ""

# 1. Prepare generated user_id for this run
echo -e "${YELLOW}[1/5] Preparing generated user_id for this run${NC}"
if [[ -z "$USER_ID" ]]; then echo -e "${RED}✗ User not found${NC}"; exit 1; fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# 2. Initial Active Purchase
echo -e "${YELLOW}[1/5] Initial Purchase (Active)${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

# Register purchase
curl -s -o /dev/null -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-notif-01-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-notif-01-reg-$RUN_ID\"
  }"

curl -s -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

# 3. Simulate Account Hold (Payment Failure)
echo -e "${YELLOW}[2/5] Triggering Account Hold (Payment Failure)${NC}"
WEBHOOK_ID="wh-notif01-hold-$(date +%s)"
TIMESTAMP=$(date +%s%3N)
NOTIFICATION_JSON="{\"version\":\"1.0\",\"packageName\":\"$PACKAGE_NAME\",\"eventTimeMillis\":\"$TIMESTAMP\",\"subscriptionNotification\":{\"version\":\"1.0\",\"notificationType\":5,\"purchaseToken\":\"$DUMMY_TOKEN\",\"subscriptionId\":\"$PRODUCT_ID\"}}"
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{\"message\":{\"data\":\"$NOTIFICATION_B64\",\"message_id\":\"$WEBHOOK_ID\"},\"subscription\":\"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"}" > /dev/null

sleep 2

# 4. Verify Notification Flag (TRUE)
echo -e "${YELLOW}[3/5] Verifying Notification Flag = TRUE${NC}"
STATUS_RESP=$(curl -s -X GET "$BRIDGE_API_URL/api/v1/subscriptions?external_user_id=$USER_ID" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0")
FLAG=$(echo "$STATUS_RESP" | jq -r '.subscriptions[0].payment_failure_notification')

if [[ "$FLAG" == "true" ]]; then
    echo -e "${GREEN}✓ Notification active${NC}"
else
    echo -e "${RED}✗ Notification NOT active (Expected true, got $FLAG)${NC}"
    exit 1
fi

# 5. Acknowledge
echo -e "${YELLOW}[4/5] Acknowledging Notification${NC}"
ACK_RESP=$(curl -s -X POST "$BRIDGE_API_URL/api/v1/subscriptions/$PRODUCT_ID/acknowledge" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0" \
  -d "{\"external_user_id\": \"$USER_ID\"}")

SUCCESS=$(echo "$ACK_RESP" | jq -r '.success')
if [[ "$SUCCESS" == "true" ]]; then
    echo -e "${GREEN} Acknowledged successfully${NC}"
else
    echo -e "${RED} Acknowledge failed: $ACK_RESP${NC}"
    exit 1
fi

# 6. Verify Notification Flag (FALSE)
echo -e "${YELLOW}[5/5] Verifying Notification Flag = FALSE${NC}"
STATUS_RESP_FINAL=$(curl -s -X GET "$BRIDGE_API_URL/api/v1/subscriptions?external_user_id=$USER_ID" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0")
FLAG_FINAL=$(echo "$STATUS_RESP_FINAL" | jq -r '.subscriptions[0].payment_failure_notification')

if [[ "$FLAG_FINAL" == "false" || "$FLAG_FINAL" == "null" ]]; then
    echo -e "${GREEN} Notification cleared${NC}"
else
    echo -e "${RED} Notification NOT cleared (Expected false/null, got $FLAG_FINAL)${NC}"
    exit 1
fi

# Report
cat > notif-01-report.json <<EOF
{
  "test_id": "NOTIF-01",
  "test_name": "Payment Failure & Acknowledgment",
  "status": "pass",
  "user_id": "$USER_ID"
}
EOF
echo -e "${GREEN}✓ NOTIF-01 Test PASSED${NC}"
exit 0
