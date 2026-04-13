#!/bin/bash

##############################################################################
# OTP-RTDN-01: Webhook Purchase Completed Test
# 
# Purpose: Verify that a ONE_TIME_PRODUCT_PURCHASED webhook is properly 
#          received, validated, and processed by the backend.
#
# Usage: ./test-otp-rtdn-01.sh --email "user@example.com" \
#                                [--token "purchase_token"] [--replay [fixture_file]]
#
# Prerequisites:
#   - OTP-01 test already completed (payment record created)
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - GOOGLE_VERIFY_WEBHOOK_SIGNATURE=false (for testing)
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

# Defaults
EMAIL=""
PURCHASE_TOKEN=""
APP_URL="$BRIDGE_API_URL"
DB_URL="$BRIDGE_DB_URL"
REPLAY_RTDN=false
REPLAY_FIXTURE=""
MOCK_GOOGLE_PURCHASE_RESPONSE=""
OTP_01_REPORT="otp-01-report.json"

if [[ ! -f "$OTP_01_REPORT" && -f "$SCRIPT_DIR/otp-01-report.json" ]]; then
    OTP_01_REPORT="$SCRIPT_DIR/otp-01-report.json"
fi

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
        --token)
            PURCHASE_TOKEN="$2"
            shift 2
            ;;
        --replay)
            REPLAY_RTDN=true
            if [[ -n "${2:-}" && "${2:0:2}" != "--" ]]; then
                REPLAY_FIXTURE="$2"
                shift 2
            else
                shift 1
            fi
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
    echo "Usage: ./test-otp-rtdn-01.sh --email \"user@example.com\" [--token \"purchase_token\"] [--replay [fixture_path]]"
    exit 1
fi

if [[ "$REPLAY_RTDN" == "true" ]]; then
    if [[ -n "$REPLAY_FIXTURE" ]]; then
        MOCK_GOOGLE_PURCHASE_RESPONSE="$REPLAY_FIXTURE"
    elif [[ -z "${MOCK_GOOGLE_PURCHASE_RESPONSE:-}" ]]; then
        MOCK_GOOGLE_PURCHASE_RESPONSE="tests/gpb/fixtures/otp-01-rtdn-purchased.json"
    fi
    echo -e "${YELLOW}[Replay] MOCK_GOOGLE_PURCHASE_RESPONSE=${MOCK_GOOGLE_PURCHASE_RESPONSE}${NC}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "OTP-RTDN-01: Webhook Purchase Completed"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Resolve user_id from OTP-01 output or the environment
echo -e "${YELLOW}[1/5] Resolving user_id for email: $EMAIL${NC}"

if [[ -z "${USER_ID:-}" && -f "$OTP_01_REPORT" ]]; then
    USER_ID=$(python3 -c "import json, sys; print(json.load(open(sys.argv[1])).get('user_id', ''))" "$OTP_01_REPORT" 2>/dev/null || echo "")
fi

if [[ -z "${USER_ID:-}" ]]; then
    echo -e "${RED}✗ Failed to resolve user_id${NC}"
    echo "Error: Run OTP-01 first in this directory, or set USER_ID before running OTP-RTDN-01"
    exit 1
fi

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 1.5: Clean up stale payment records (but preserve OTP-01's token)
echo -e "${YELLOW}[1.5/5] Cleaning up stale payment records${NC}"
PAYMENT_CLEANUP="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider_transaction_id != 'test-inapp-otp-01-12345';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_CLEANUP" 2>/dev/null
echo -e "${GREEN}✓ Stale payment records removed (preserved OTP-01 token)${NC}"
echo ""

# Step 2: Verify payment record exists
echo -e "${YELLOW}[2/5] Verifying payment record exists (from OTP-01 or similar)${NC}"

DB_QUERY="SELECT status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

echo "Query:"
echo "  $DB_QUERY"
echo ""

DB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$DB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$DB_RESULT" || "$DB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in database${NC}"
    echo "Error: Please run OTP-01 test first to create a payment record"
    echo "Database result: $DB_RESULT"
    exit 1
fi

echo -e "${GREEN}✓ Payment record found:${NC}"
echo "$DB_RESULT" | while read line; do
    echo "  $line"
done
echo ""

# Extract values
STORED_STATUS=$(echo "$DB_RESULT" | awk -F '|' '{print $1}' | head -n1 | tr -d ' ')
STORED_TOKEN=$(echo "$DB_RESULT" | awk -F '|' '{print $2}' | head -n1 | tr -d ' ')

if [[ -z "$PURCHASE_TOKEN" ]]; then
    PURCHASE_TOKEN=$STORED_TOKEN
fi

echo -e "${BLUE}Current status: $STORED_STATUS${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Send mock ONE_TIME_PRODUCT_PURCHASED webhook
echo -e "${YELLOW}[3/5] Sending ONE_TIME_PRODUCT_PURCHASED webhook${NC}"
echo ""

# Generate current timestamp in milliseconds and MESSAGE_ID once (for idempotency testing)
TIMESTAMP=$(date +%s000)
MESSAGE_ID="webhook-purchase-rtdn-01-$(date +%s)"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  Product ID: $PRODUCT_ID"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo "  Timestamp: $TIMESTAMP"
echo ""
echo "Backend will call: purchases.products.get() (v1)"
echo "  (NOT pay.subscriptions API which is for pay.subscriptions)"
echo ""

# Create DeveloperNotification JSON (the actual notification)
# Note: Google sends "sku" not "productId" in real webhooks
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "oneTimeProductNotification": {
    "version": "1.0",
    "notificationType": 1,
    "purchaseToken": "$PURCHASE_TOKEN",
    "sku": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send mock purchase notification webhook as PubSub message
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
if [[ ! -z "$WEBHOOK_BODY" ]]; then
    echo "Webhook Response: $WEBHOOK_BODY"
fi
echo ""

if [[ "$WEBHOOK_HTTP_CODE" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Purchase webhook accepted (HTTP $WEBHOOK_HTTP_CODE)${NC}"
    WEBHOOK_ACCEPTED="true"
else
    echo -e "${YELLOW}⚠ Webhook returned HTTP $WEBHOOK_HTTP_CODE${NC}"
    WEBHOOK_ACCEPTED="false"
fi
echo ""

# Step 4: Verify idempotency (send same webhook again)
echo -e "${YELLOW}[4/5] Testing idempotency (send same webhook again)${NC}"
echo ""

WEBHOOK_RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST \
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

WEBHOOK_HTTP_CODE_2=$(echo "$WEBHOOK_RESPONSE_2" | tail -n1)

if [[ "$WEBHOOK_HTTP_CODE_2" == "200" ]] || [[ "$WEBHOOK_HTTP_CODE_2" == "204" ]]; then
    echo -e "${GREEN}✓ Duplicate webhook also accepted (HTTP $WEBHOOK_HTTP_CODE_2, idempotent)${NC}"
    IDEMPOTENCY_WORKS="true"
else
    echo -e "${YELLOW}⚠ Duplicate webhook returned HTTP $WEBHOOK_HTTP_CODE_2${NC}"
    IDEMPOTENCY_WORKS="false"
fi
echo ""

# Step 5: Final verification (idempotency + record counts)
echo -e "${YELLOW}[5/5] Final verification${NC}"

# Query payment status to verify it's still success
FINAL_DB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')

echo "Final payment status: $FINAL_DB_STATUS"
echo ""

if [[ "$FINAL_DB_STATUS" == "success" ]]; then
    echo -e "${GREEN}✓ Payment status remains success${NC}"
    STATUS_VERIFIED="true"
else
    echo -e "${YELLOW}⚠ Status is '$FINAL_DB_STATUS' (expected 'success')${NC}"
    STATUS_VERIFIED="false"
fi
echo ""

# Verify payment record exists and count is exactly 1 (idempotency check)
PAYMENT_COUNT_QUERY="SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_COUNT_QUERY" -t 2>/dev/null | tr -d ' ' || echo "0")

echo "Payment record count: $PAYMENT_COUNT"

if [[ "$PAYMENT_COUNT" == "0" ]]; then
    echo -e "${YELLOW}⚠ No payment record found${NC}"
    PAYMENT_VERIFIED="false"
elif [[ "$PAYMENT_COUNT" != "1" ]]; then
    echo -e "${RED}✗ Idempotency violation: Expected 1 payment record (duplicate webhooks), got $PAYMENT_COUNT${NC}"
    PAYMENT_VERIFIED="false"
    IDEMPOTENCY_WORKS="false"
else
    # Count is 1, verify status
    PAYMENT_QUERY="SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"
    PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null | tr -d ' ')
    PAYMENT_STATUS=$(echo "$PAYMENT_RESULT" | tr -d ' ')
    echo -e "${GREEN}✓ Payment record count verified (exactly 1, idempotent)${NC}"
    echo -e "${GREEN}✓ Payment record status: $PAYMENT_STATUS${NC}"
    PAYMENT_VERIFIED="true"
fi
echo ""

# Determine test status
TEST_STATUS="pass"
if [[ "$STATUS_VERIFIED" != "true" ]] || [[ "$IDEMPOTENCY_WORKS" != "true" ]] || [[ "$WEBHOOK_ACCEPTED" != "true" ]]; then
    TEST_STATUS="fail"
fi

# Generate JSON report
cat > otp-rtdn-01-report.json <<EOF
{
  "test_id": "OTP-RTDN-01",
  "test_name": "Webhook Purchase Completed",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "message_id": "$MESSAGE_ID",
  "webhook_accepted": $WEBHOOK_ACCEPTED,
  "idempotency_verified": $IDEMPOTENCY_WORKS,
  "final_status": "$FINAL_DB_STATUS",
  "results": {
    "webhook_http_success": $WEBHOOK_ACCEPTED,
    "webhook_accepted_successfully": $WEBHOOK_ACCEPTED,
    "duplicate_webhook_idempotent": $IDEMPOTENCY_WORKS,
    "subscription_status_active": $([ "$FINAL_DB_STATUS" == "active" ] && echo "true" || echo "false"),
    "payment_record_exists": $PAYMENT_VERIFIED
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ OTP-RTDN-01 Test PASSED${NC}"
else
    echo -e "${RED}✗ OTP-RTDN-01 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: otp-rtdn-01-report.json"
cat otp-rtdn-01-report.json
echo ""

if [[ "$TEST_STATUS" != "pass" ]]; then
    exit 1
fi
exit 0
