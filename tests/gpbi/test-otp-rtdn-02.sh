#!/bin/bash

##############################################################################
# OTP-RTDN-02: Webhook Refund Completed (One-Time Product)
# 
# Purpose: Verify that a voidedPurchaseNotification (refund) is 
#          properly received, validated, and revokes entitlement.
#
# Usage: ./test-otp-rtdn-02.sh [--token "purchase_token"] [--replay [fixture_file]]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_OTP
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: 'voidedPurchaseNotification' webhook arrives for an existing OTP purchase.
#                      Backend processes the webhook and returns HTTP 200/204.
#                      Payment record in pay.payments transitions to 'refunded'.
#                      Subsequent identical webhooks (same message_id) are handled idempotently via webhook_log.
#                      Ensures revenue and access are correctly adjusted after refund via RTDN.
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

if [[ "$REPLAY_RTDN" == "true" ]]; then
    if [[ -n "$REPLAY_FIXTURE" ]]; then
        MOCK_GOOGLE_PURCHASE_RESPONSE="$REPLAY_FIXTURE"
    elif [[ -z "${MOCK_GOOGLE_PURCHASE_RESPONSE:-}" ]]; then
        MOCK_GOOGLE_PURCHASE_RESPONSE="tests/gpb/fixtures/otp-05-rtdn-refunded.json"
    fi
    echo -e "${YELLOW}[Replay] MOCK_GOOGLE_PURCHASE_RESPONSE=${MOCK_GOOGLE_PURCHASE_RESPONSE}${NC}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "OTP-RTDN-02: Webhook Refund Completed"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_otp_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"

if [[ -z "$PURCHASE_TOKEN" && -f "$OTP_01_REPORT" ]]; then
    PURCHASE_TOKEN=$(python3 -c "import json, sys; print(json.load(open(sys.argv[1])).get('purchase_token', ''))" "$OTP_01_REPORT" 2>/dev/null || echo "")
fi
echo ""

# Step 1.5: Clean up stale payment records (but preserve OTP-01's token)
echo -e "${YELLOW}[1.5/6] Cleaning up stale payment records${NC}"
if [[ -n "$PURCHASE_TOKEN" ]]; then
    PAYMENT_CLEANUP="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider_transaction_id != '$PURCHASE_TOKEN';"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_CLEANUP" 2>/dev/null
    echo -e "${GREEN}✓ Stale payment records removed (preserved OTP-01 token)${NC}"
else
    echo -e "${YELLOW}⚠ Skipped stale payment cleanup (no OTP-01 token supplied or found)${NC}"
fi
echo ""

# Step 2: Verify payment record exists
echo -e "${YELLOW}[2/6] Verifying payment record exists${NC}"

if [[ -n "$PURCHASE_TOKEN" ]]; then
    DB_QUERY="SELECT status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND provider_transaction_id = '$PURCHASE_TOKEN' ORDER BY created_at DESC LIMIT 1;"
else
    DB_QUERY="SELECT status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"
fi

echo "Query:"
echo "  $DB_QUERY"
echo ""

DB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$DB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$DB_RESULT" || "$DB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in database${NC}"
    echo "Error: Please run OTP-01 test first to create a payment record, or pass --token"
    echo "Database result: $DB_RESULT"
    exit 1
fi

echo -e "${GREEN}✓ Payment record found:${NC}"
echo "$DB_RESULT" | while read line; do
    echo "  $line"
done
echo ""

# Extract values
INITIAL_STATUS=$(echo "$DB_RESULT" | awk -F '|' '{print $1}' | head -n1 | tr -d ' ')
STORED_TOKEN=$(echo "$DB_RESULT" | awk -F '|' '{print $2}' | head -n1 | tr -d ' ')

if [[ "$INITIAL_STATUS" != "success" ]]; then
    echo -e "${YELLOW}⚠ Warning: Initial status is '$INITIAL_STATUS', expected 'success'${NC}"
fi

if [[ -z "$PURCHASE_TOKEN" ]]; then
    PURCHASE_TOKEN=$STORED_TOKEN
fi

echo -e "${BLUE}Initial status: $INITIAL_STATUS${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 3: Send mock ONE_TIME_PRODUCT_CANCELED webhook (refund)
echo -e "${YELLOW}[3/6] Sending ONE_TIME_PRODUCT_CANCELED webhook (refund notification)${NC}"
echo ""

# Generate current timestamp in milliseconds
TIMESTAMP=$(date +%s000)
MESSAGE_ID="webhook-refund-$(date +%s)-$RANDOM"

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
# Use voidedPurchaseNotification (refund) not oneTimeProductNotification (cancellation)
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "voidedPurchaseNotification": {
    "purchaseToken": "$PURCHASE_TOKEN",
    "orderId": "GPA.1111-2222-3333-44444",
    "productType": 0,
    "refundType": 0
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send mock refund/cancellation notification webhook as PubSub message
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
    echo -e "${GREEN}✓ Refund webhook accepted (HTTP $WEBHOOK_HTTP_CODE)${NC}"
    WEBHOOK_ACCEPTED="true"
else
    echo -e "${RED}✗ Webhook rejected with HTTP $WEBHOOK_HTTP_CODE${NC}"
    exit 1
fi
echo ""

# Wait for async webhook processing to complete (backend processes asynchronously)
sleep 1
echo -e "${BLUE}Waiting for async webhook processing...${NC}"
echo ""

# Step 4: Verify idempotency (send same webhook again)
echo -e "${YELLOW}[4/6] Testing idempotency (send same refund webhook again)${NC}"
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
    echo -e "${GREEN}✓ Duplicate refund webhook also accepted (HTTP $WEBHOOK_HTTP_CODE_2, idempotent)${NC}"
    IDEMPOTENCY_WORKS="true"
else
    echo -e "${YELLOW}⚠ Duplicate webhook returned HTTP $WEBHOOK_HTTP_CODE_2${NC}"
    IDEMPOTENCY_WORKS="false"
fi
echo ""

# Step 5: Verify payment record updated to refunded status (with idempotency)
echo -e "${YELLOW}[5/6] Verifying payment record status updated to refunded (idempotency)${NC}"

# First verify count is exactly 1 (idempotency check)
PAYMENT_COUNT_QUERY="SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
PAYMENT_QUERY="SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"
PAYMENT_COUNT="0"
PAYMENT_STATUS=""

for attempt in $(seq 1 $WEBHOOK_WAIT_ATTEMPTS); do
    PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_COUNT_QUERY" -t 2>/dev/null | tr -d ' ' || echo "0")
    PAYMENT_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null | tr -d ' ')

    if [[ "$PAYMENT_COUNT" == "1" ]] && [[ "$PAYMENT_STATUS" == "refunded" ]]; then
        break
    fi

    if [[ $attempt -lt $WEBHOOK_WAIT_ATTEMPTS ]]; then
        sleep $WEBHOOK_WAIT_SECONDS
    fi
done

echo "Payment record count: $PAYMENT_COUNT"

if [[ "$PAYMENT_COUNT" == "0" ]]; then
    echo -e "${RED}✗ No payment record found${NC}"
    PAYMENT_REFUNDED="false"
elif [[ "$PAYMENT_COUNT" != "1" ]]; then
    echo -e "${RED}✗ Idempotency violation: Expected 1 payment record (duplicate webhooks), got $PAYMENT_COUNT${NC}"
    PAYMENT_REFUNDED="false"
    IDEMPOTENCY_WORKS="false"
else
    echo -e "${GREEN}✓ Payment record count verified (exactly 1, idempotent)${NC}"
    echo -e "${GREEN}✓ Payment record status: $PAYMENT_STATUS${NC}"
    
    if [[ "$PAYMENT_STATUS" == "refunded" ]] || [[ "$PAYMENT_STATUS" == "cancelled" ]]; then
        PAYMENT_REFUNDED="true"
    else
        PAYMENT_REFUNDED="false"
    fi
fi
echo ""

# Step 6: Final verification - check payment status changed
echo -e "${YELLOW}[6/6] Final verification - checking payment status${NC}"

FINAL_DB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')

echo "Status transition:"
echo "  Before refund: $INITIAL_STATUS"
echo "  After refund:  $FINAL_DB_STATUS"
echo ""

if [[ "$FINAL_DB_STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Refund properly reflected in payment record${NC}"
    STATUS_REVOKED="true"
else
    echo -e "${RED}✗ Expected status 'refunded', got '$FINAL_DB_STATUS'${NC}"
    STATUS_REVOKED="false"
fi
echo ""

# Determine test status
TEST_STATUS="pass"
if [[ "$STATUS_REVOKED" != "true" ]] || [[ "$PAYMENT_REFUNDED" != "true" ]] || [[ "$IDEMPOTENCY_WORKS" != "true" ]] || [[ "$WEBHOOK_ACCEPTED" != "true" ]]; then
    TEST_STATUS="fail"
fi

# Generate JSON report
cat > otp-rtdn-02-report.json <<EOF
{
  "test_id": "OTP-RTDN-02",
  "test_name": "Webhook Refund Completed",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "message_id": "$MESSAGE_ID",
  "webhook_accepted": $WEBHOOK_ACCEPTED,
  "idempotency_verified": $IDEMPOTENCY_WORKS,
  "initial_status": "$INITIAL_STATUS",
  "final_status": "$FINAL_DB_STATUS",
  "results": {
    "webhook_http_success": $WEBHOOK_ACCEPTED,
    "webhook_accepted_successfully": $WEBHOOK_ACCEPTED,
    "duplicate_webhook_idempotent": $IDEMPOTENCY_WORKS,
    "subscription_status_revoked": $STATUS_REVOKED,
    "payment_status_refunded": $PAYMENT_REFUNDED,
    "entitlement_revoked": $STATUS_REVOKED
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ OTP-RTDN-02 Test PASSED${NC}"
else
    echo -e "${RED}✗ OTP-RTDN-02 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: otp-rtdn-02-report.json"
cat otp-rtdn-02-report.json
echo ""

if [[ "$TEST_STATUS" != "pass" ]]; then
    exit 1
fi
exit 0
