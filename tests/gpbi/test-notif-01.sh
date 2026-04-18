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
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"
USER_ID="${USER_ID:-test_notif_01_user_$RUN_ID}"

export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

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
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
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
TIMESTAMP=$(date +%s000)
NOTIFICATION_JSON="{\"version\":\"1.0\",\"packageName\":\"$PACKAGE_NAME\",\"eventTimeMillis\":\"$TIMESTAMP\",\"subscriptionNotification\":{\"version\":\"1.0\",\"notificationType\":5,\"purchaseToken\":\"$DUMMY_TOKEN\",\"subscriptionId\":\"$PRODUCT_ID\"}}"
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{\"message\":{\"data\":\"$NOTIFICATION_B64\",\"message_id\":\"$WEBHOOK_ID\"},\"subscription\":\"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"}" > /dev/null

sleep 2

# 4. Verify Notification Flag (TRUE)
echo -e "${YELLOW}[3/5] Verifying Notification Flag = TRUE${NC}"
STATUS_RESP=$(curl -s -H "Authorization: Bearer $API_KEY" -X GET "$APP_URL/api/v1/pay.subscriptions"   -H "x-client-version: 99.99.0")
FLAG=$(echo "$STATUS_RESP" | jq -r '.payment_failure_notification')

if [[ "$FLAG" == "true" ]]; then
    echo -e "${GREEN}✓ Notification active${NC}"
else
    echo -e "${RED}✗ Notification NOT active (Expected true, got $FLAG)${NC}"
    exit 1
fi

# 5. Acknowledge
echo -e "${YELLOW}[4/5] Acknowledging Notification${NC}"
ACK_RESP=$(curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/notifications/payment-failure/acknowledge" \
  -H "Content-Type: application/json" \
   \
   \
  -H "x-client-version: 99.99.0" \
  -d "{\"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\"}")

STATUS=$(echo "$ACK_RESP" | jq -r '.status')
if [[ "$STATUS" == "acknowledged" ]]; then
    echo -e "${GREEN}✓ Acknowledged successfully${NC}"
else
    echo -e "${RED}✗ Acknowledge failed: $ACK_RESP${NC}"
    exit 1
fi

# 6. Verify Notification Flag (FALSE)
echo -e "${YELLOW}[5/5] Verifying Notification Flag = FALSE${NC}"
STATUS_RESP_FINAL=$(curl -s -H "Authorization: Bearer $API_KEY" -X GET "$APP_URL/api/v1/pay.subscriptions"   -H "x-client-version: 99.99.0")
FLAG_FINAL=$(echo "$STATUS_RESP_FINAL" | jq -r '.payment_failure_notification')

if [[ "$FLAG_FINAL" == "false" || "$FLAG_FINAL" == "null" ]]; then
    echo -e "${GREEN}✓ Notification cleared${NC}"
else
    echo -e "${RED}✗ Notification NOT cleared (Expected false/null, got $FLAG_FINAL)${NC}"
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
