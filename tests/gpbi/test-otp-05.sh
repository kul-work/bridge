#!/bin/bash

##############################################################################
# OTP-05: Refund After Purchase Test
# 
# Purpose: Verify that a refunded OTP purchase is detected and the payment 
#          status changes to 'refunded' via webhook simulation.
#
# Usage: ./test-otp-05.sh \
#                        [--token "purchase_token"]
#
# Prerequisites:
#   - OTP-01 test already completed (payment record created)
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - Google webhook signature verification disabled via test header
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
PRODUCT_ID="$PRODUCT_ID_OTP"
PROVIDER="$PROVIDER"
WEBHOOK_WAIT_ATTEMPTS=10
WEBHOOK_WAIT_SECONDS=1

# Defaults
PURCHASE_TOKEN=""
APP_URL="$BRIDGE_API_URL"
DB_URL="$BRIDGE_DB_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --token)
            PURCHASE_TOKEN="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "OTP-05: Refund After Purchase Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_otp_user_05"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing entries from previous tests
echo -e "${YELLOW}[2/6] Cleaning up previous test data${NC}"

CLEANUP_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous subscription record removed${NC}"

CLEANUP_PAYMENTS_QUERY="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_PAYMENTS_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous payment records removed${NC}"
echo ""

# Step 3: Verify payment record exists with success status
# Generate unique token for this test run to avoid collisions
DUMMY_TOKEN="test-inapp-otp-05-$(date +%s)"
PURCHASE_TOKEN=$DUMMY_TOKEN

echo -e "${YELLOW}[3/6] Setup: Creating fresh purchase to refund${NC}"
echo "  Token: $PURCHASE_TOKEN"

# Perform Setup Purchase
VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"inapp\"
  }")

SETUP_HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
if [[ "$SETUP_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ Setup purchase failed with HTTP $SETUP_HTTP_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Setup purchase successful (HTTP 200)${NC}"

# Verify initial DB state
DB_QUERY="SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider_transaction_id = '$PURCHASE_TOKEN';"
INITIAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$DB_QUERY" -t 2>/dev/null | tr -d ' ')

if [[ "$INITIAL_STATUS" != "success" ]]; then
    echo -e "${RED}✗ Setup failed: Initial status is '$INITIAL_STATUS', expected 'success'${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Initial payment status: $INITIAL_STATUS${NC}"
echo ""

# Step 3: Simulate refund in Play Console (manual action)
echo -e "${YELLOW}[3/6] Simulating refund from Play Console${NC}"
echo ""
echo "Sending mock webhook to backend..."
echo ""

# Generate timestamp and message ID
TIMESTAMP=$(date +%s000)
MESSAGE_ID="webhook-otp05-refund-$(date +%s)-$RANDOM"

# Create DeveloperNotification JSON (the actual notification)  
NOTIFICATION_JSON="{\"version\":\"1.0\",\"packageName\":\"$PACKAGE_NAME\",\"eventTimeMillis\":\"$TIMESTAMP\",\"voidedPurchaseNotification\":{\"purchaseToken\":\"$PURCHASE_TOKEN\",\"orderId\":\"GPA.1111-2222-3333-44444\",\"productType\":0,\"refundType\":0}}"

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send as PubSub message format
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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
WEBHOOK_LINE_COUNT=$(echo "$WEBHOOK_RESPONSE" | wc -l)
if [ "$WEBHOOK_LINE_COUNT" -gt 1 ]; then
    WEBHOOK_BODY=$(echo "$WEBHOOK_RESPONSE" | head -n $((WEBHOOK_LINE_COUNT - 1)))
else
    WEBHOOK_BODY=""
fi

echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"
echo "Webhook Response: $WEBHOOK_BODY"
echo ""

if [[ "$WEBHOOK_HTTP_CODE" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Refund webhook accepted (HTTP $WEBHOOK_HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook rejected with HTTP $WEBHOOK_HTTP_CODE${NC}"
    exit 1
fi
echo ""

# Step 4: Wait for async webhook processing
echo -e "${YELLOW}[4/6] Waiting for webhook processing${NC}"
echo ""
echo "Polling payment record until refund is applied..."
echo ""

# Step 5: Verify payment record in pay.payments table with refunded status
echo -e "${YELLOW}[5/7] Verifying payment record has refunded status${NC}"

PAYMENT_QUERY="SELECT amount_cents, status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

echo "Query:"
echo "  $PAYMENT_QUERY"
echo ""

PAYMENT_RESULT=""
PAYMENT_STATUS=""
PAYMENT_AMOUNT=""
PAYMENT_TXN_ID=""

for attempt in $(seq 1 $WEBHOOK_WAIT_ATTEMPTS); do
    PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

    if [[ -n "$PAYMENT_RESULT" && "$PAYMENT_RESULT" != *"(0 rows)"* ]]; then
        PAYMENT_AMOUNT=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $1}' | tr -d ' ')
        PAYMENT_STATUS=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $2}' | tr -d ' ')
        PAYMENT_TXN_ID=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $3}' | tr -d ' ')

        if [[ "$PAYMENT_STATUS" == "refunded" ]]; then
            break
        fi
    fi

    if [[ $attempt -lt $WEBHOOK_WAIT_ATTEMPTS ]]; then
        sleep $WEBHOOK_WAIT_SECONDS
    fi
done

if [[ -z "$PAYMENT_RESULT" || "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in pay.payments table${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Payment Record Found:${NC}"
echo "  Amount: ${PAYMENT_AMOUNT} cents"
echo "  Status: ${PAYMENT_STATUS}"
echo "  Transaction ID: ${PAYMENT_TXN_ID}"
echo ""

# Verify status is refunded
if [[ "$PAYMENT_STATUS" != "refunded" ]]; then
    echo -e "${RED}✗ Expected payment status 'refunded', got '$PAYMENT_STATUS'${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Payment status correctly set to 'refunded'${NC}"
echo ""

# Step 6: Verify status change via /api/v1/subscriptions
echo -e "${YELLOW}[6/7] Checking status via /api/v1/subscriptions endpoint${NC}"

STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
  "$APP_URL/api/v1/subscriptions" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  )

STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
LINE_COUNT=$(echo "$STATUS_RESPONSE" | wc -l)
if [ "$LINE_COUNT" -gt 1 ]; then
    STATUS_BODY=$(echo "$STATUS_RESPONSE" | head -n $((LINE_COUNT - 1)))
else
    STATUS_BODY=""
fi

echo "Response Code: $STATUS_HTTP_CODE"
echo "Response: $STATUS_BODY"
echo ""

if [[ "$STATUS_HTTP_CODE" != "200" ]]; then
    echo -e "${YELLOW}⚠ subscription-status returned HTTP $STATUS_HTTP_CODE${NC}"
fi
echo ""

# Step 7: Final verification
echo -e "${YELLOW}[7/7] Final verification${NC}"

FINAL_DB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')

echo "Status transition:"
echo "  Before refund: $INITIAL_STATUS"
echo "  After refund:  $FINAL_DB_STATUS"
echo ""

if [[ "$FINAL_DB_STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Refund properly reflected in payment record${NC}"
    REFUND_DETECTED="true"
else
    echo -e "${RED}✗ Expected status 'refunded', got '$FINAL_DB_STATUS'${NC}"
    REFUND_DETECTED="false"
fi
echo ""

# Generate JSON report
cat > otp-05-report.json <<EOF
{
  "test_id": "OTP-05",
  "test_name": "Refund After Purchase",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "initial_status": "$INITIAL_STATUS",
  "final_status": "$FINAL_DB_STATUS",
  "refund_detected": $REFUND_DETECTED,
  "results": {
    "subscription_existed": true,
    "refund_action_simulated": true,
    "status_change_detected": $([ "$REFUND_DETECTED" == "true" ] && echo "true" || echo "false"),
    "is_cancelled_or_expired": $([ "$FINAL_DB_STATUS" == "cancelled" ] || [ "$FINAL_DB_STATUS" == "expired" ] && echo "true" || echo "false")
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ OTP-05 Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: otp-05-report.json"
cat otp-05-report.json
echo ""

exit 0
