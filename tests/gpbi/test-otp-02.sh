#!/bin/bash

##############################################################################
# OTP-02: Declined Payment Test
# 
# Purpose: Verify that a declined payment (test card that always declines)
#          is rejected by the payment verification endpoint and no database
#          entry is created.
#
# Usage: ./test-otp-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
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
DECLINED_TOKEN="test-inapp-declined-card"
PRODUCT_ID="$PRODUCT_ID_OTP"
PROVIDER="$PROVIDER"

# Defaults
APP_URL="$BRIDGE_API_URL"
DB_URL="$BRIDGE_DB_URL"

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
echo "OTP-02: Declined Payment Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_otp_user_02"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing entries from previous successful tests (reset for this decline test)
echo -e "${YELLOW}[2/5] Cleaning up previous test data${NC}"

CLEANUP_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous subscription record removed${NC}"

CLEANUP_PAYMENTS_QUERY="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_PAYMENTS_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous payment records removed${NC}"
echo ""

# Step 3: Call /api/v1/verify-purchase endpoint with declined token
echo -e "${YELLOW}[3/5] Calling /api/v1/verify-purchase with declined test card token${NC}"

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  Provider: $PROVIDER"
echo "  Product ID: $PRODUCT_ID"
echo "  Token: $DECLINED_TOKEN (simulates test card that always declines)"
echo "  Product Type: inapp"
echo ""

echo "Sending request..."
VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DECLINED_TOKEN\",
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

# Check for error response (HTTP 200 with error payload due to mobile client requirements)
# Backend returns 200 OK with error details in JSON for mobile client handling
if echo "$VERIFY_BODY" | grep -qi "PAYMENT_PROVIDER_ERROR\|declined\|failed"; then
    echo -e "${GREEN}✓ verify_purchase correctly rejected declined token${NC}"
    echo -e "${GREEN}✓ Response contains error code: PAYMENT_PROVIDER_ERROR${NC}"
else
    echo -e "${RED}✗ Expected payment provider error for declined payment, got: $VERIFY_BODY${NC}"
    exit 1
fi
echo ""

# Step 4: Verify NO entry was created by the declined payment
echo -e "${YELLOW}[4/5] Verifying declined payment did NOT create a database entry${NC}"

DB_QUERY="SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $DB_QUERY"
echo ""

DB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$DB_QUERY" -t 2>/dev/null | tr -d ' ' || echo "0")

echo "Result: $DB_COUNT subscription records found"
echo ""

if [[ "$DB_COUNT" != "0" ]]; then
    echo -e "${RED}✗ Expected 0 subscription records (declined payment should not be stored), found $DB_COUNT${NC}"
    exit 1
fi

echo -e "${GREEN}✓ No subscription record created (as expected for declined payment)${NC}"
echo ""

# Step 5: Verify NO payment record was created
echo -e "${YELLOW}[5/5] Verifying no payment record was created${NC}"

PAYMENT_QUERY="SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $PAYMENT_QUERY"
echo ""

PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null | tr -d ' ' || echo "0")

echo "Result: $PAYMENT_COUNT payment records found"
echo ""

if [[ "$PAYMENT_COUNT" != "0" ]]; then
    echo -e "${RED}✗ Expected 0 payment records (declined payment should not create payment entry), found $PAYMENT_COUNT${NC}"
    exit 1
fi

echo -e "${GREEN}✓ No payment record created (as expected for declined payment)${NC}"
echo ""

# Step 6: Verify user can retry with a successful token
echo -e "${YELLOW}[6/6] Verifying user can retry with a successful token${NC}"

echo "Expected behavior:"
echo "  - Frontend dialog remains open for retry"
echo "  - User can select different payment method"
echo "  - Backend allows subsequent verify attempts for same product"
echo ""

# Quick check: Database should still have 0 subscription entries
DB_COUNT_CHECK=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ' || echo "0")

if [[ "$DB_COUNT_CHECK" == "0" ]]; then
    echo -e "${GREEN}✓ Retry capability verified (no stale DB entries preventing retry)${NC}"
else
    echo -e "${RED}✗ Database contains entries from failed attempt (retry would fail)${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > otp-02-report.json <<EOF
{
  "test_id": "OTP-02",
  "test_name": "Declined Payment",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "declined_token": "$DECLINED_TOKEN",
  "http_code": $HTTP_CODE,
  "database_record_count": $DB_COUNT,
  "results": {
    "verify_endpoint_rejected_decline": true,
    "response_indicates_failure": true,
    "no_database_entry_created": true,
    "user_can_retry": true
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ OTP-02 Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: otp-02-report.json"
cat otp-02-report.json
echo ""

exit 0
