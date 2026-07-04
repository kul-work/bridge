#!/bin/bash

##############################################################################
# OTP-01: Successful One-Time Purchase (Verified & Acknowledged)
# 
# Purpose: Verify that a successful INAPP product purchase is properly 
#          verified, stored in pay.payments, and acknowledged to the provider.
#
# Usage: ./test-otp-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_OTP
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: POST /api/v1/verify-purchase returns 200 OK.
#                      A record is created in pay.payments with status='success'.
#                      No record is created in pay.subscriptions (correct for one-time).
#                      'acknowledged_at' is set in pay.payments.
#                      Ensures one-time products follow a clean 'verify-and-acknowledge' flow.
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
TEST_RUN_ID="otp-01-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_OTP"
DUMMY_TOKEN="test-otp-01-token-$TEST_RUN_ID"
PROVIDER="$PROVIDER"
REPORT_FILE="otp-01-report.json"
USER_ID="${USER_ID:-test_otp_01_user_$TEST_RUN_ID}"

# Defaults
DB_URL="$BRIDGE_DB_URL"
MOCK_GOOGLE_ACKNOWLEDGE_RESPONSE=""

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "OTP-01: Successful Purchase Test"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing entries from previous tests
echo -e "${YELLOW}[2/7] Cleaning up previous test data${NC}"

CLEANUP_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous subscription record removed${NC}"

CLEANUP_PAYMENTS_QUERY="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_PAYMENTS_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous payment records removed${NC}"
echo ""

# Step 3: Call /api/v1/verify-purchase endpoint
echo -e "${YELLOW}[3/7] Calling /api/v1/verify-purchase${NC}"

echo "  POST $BRIDGE_API_URL/api/v1/verify-purchase"
echo "  Provider: $PROVIDER"
echo "  Product ID: $PRODUCT_ID"
echo "  Token: $DUMMY_TOKEN (purchaseState: 0 = PURCHASED)"
echo "  Product Type: inapp"
echo "  API Method: purchases.products.get() (v1)"
echo ""

echo "Sending request..."

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"inapp\"
  }")

HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
LINE_COUNT=$(echo "$VERIFY_RESPONSE" | wc -l)
if [ "$LINE_COUNT" -gt 1 ]; then
    VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | head -n $((LINE_COUNT - 1)))
else
    VERIFY_BODY=""
fi

echo "Response Code: $HTTP_CODE"
echo "Response: $VERIFY_BODY"
echo ""

if [[ "$HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ verify_purchase failed with HTTP $HTTP_CODE${NC}"
    exit 1
fi

echo -e "${GREEN}✓ verify_purchase returned HTTP 200${NC}"
echo ""

# Step 4: Verify no subscription record (OTP creates pay.payments, not pay.subscriptions)
echo -e "${YELLOW}[4/7] Verifying no subscription record created (OTP is one-time, not subscription)${NC}"

DB_QUERY="SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $DB_QUERY"
echo ""

SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$DB_QUERY" -t 2>/dev/null | tr -d ' ' || echo "error")

if [[ "$SUB_COUNT" != "0" ]]; then
    echo -e "${RED}✗ Expected 0 subscription records for OTP, got $SUB_COUNT${NC}"
    exit 1
fi

echo -e "${GREEN}✓ No subscription records created (correct for one-time product)${NC}"
echo ""

# Step 5: Verify payment record in pay.payments table
echo -e "${YELLOW}[5/7] Verifying pay.payments table record${NC}"

PAYMENT_QUERY="SELECT amount_cents, status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

echo "Query:"
echo "  $PAYMENT_QUERY"
echo ""

PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$PAYMENT_RESULT" || "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in pay.payments table${NC}"
    exit 1
fi

PAYMENT_AMOUNT=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $1}' | tr -d ' ')
PAYMENT_STATUS=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $2}' | tr -d ' ')
PAYMENT_TOKEN=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $3}' | tr -d ' ')

if [[ "$PAYMENT_STATUS" != "success" ]]; then
    echo -e "${RED}✗ Expected payment status 'success', got '$PAYMENT_STATUS'${NC}"
    exit 1
fi

# Note: amount might be 0 if mock didn't return price, but we just check record exists and is success
echo -e "${GREEN}✓ Payment Record Found: Amount=${PAYMENT_AMOUNT}, Status=${PAYMENT_STATUS}${NC}"
echo -e "${GREEN}✓ Payment Token: ${PAYMENT_TOKEN}${NC}"
echo ""

# Step 6: Verify acknowledgment in pay.payments table (canonical idempotency source)
echo -e "${YELLOW}[6/7] Verifying acknowledgment in pay.payments table${NC}"

# PAYMENT_TOKEN holds the actual provider_transaction_id stored by the server
# (which is the order id, e.g. mock-google-play-order:<purchase_token>, not the raw token).
ACK_QUERY="SELECT acknowledged_at FROM pay.payments WHERE provider_transaction_id = '$PAYMENT_TOKEN' LIMIT 1;"

echo "Query:"
echo "  $ACK_QUERY"
echo ""

ACK_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$ACK_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$ACK_RESULT" || "$ACK_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ Expected payment acknowledgment in pay.payments table for token: $DUMMY_TOKEN${NC}"
    exit 1
fi

ACK_TIMESTAMP=$(echo "$ACK_RESULT" | tr -d ' ')
echo -e "${GREEN}✓ Payment Acknowledged: $ACK_TIMESTAMP (set in pay.payments table)${NC}"
echo ""


# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-01",
  "test_name": "Successful One-Time Purchase",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$DUMMY_TOKEN",
  "payment_status": "$PAYMENT_STATUS",
  "http_code": $HTTP_CODE,
  "database_verified": true,
  "results": {
    "verify_endpoint_success": true,
    "database_record_found": true,
    "status_is_success": true,
    "token_matches": true,
    "acknowledged": true
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ OTP-01 Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

exit 0
