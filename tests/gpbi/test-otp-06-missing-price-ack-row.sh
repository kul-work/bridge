#!/bin/bash

##############################################################################
# OTP-06: Google One-Time Purchase Missing Price Still Creates ACK Row
#
# Purpose: Verify that a successful Google Play INAPP purchase with no explicit
#          amount/currency is still persisted in pay.payments with the identity
#          fields needed by the product ACK worker.
#
# Usage: ./test-otp-06-missing-price-ack-row.sh
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
#   Expected Behavior: POST /api/v1/verify-purchase returns 200 OK for a
#                      purchased Google one-time product even when Google does
#                      not return explicit price/currency fields. Bridge must
#                      still create a pay.payments row with provider_purchase_token,
#                      product_id, ack_required=true, and status='success' so
#                      the product ACK worker has durable identity to process.
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="otp-06-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_OTP"
DUMMY_TOKEN="test-otp-06-token-$TEST_RUN_ID"
PROVIDER="$PROVIDER"
REPORT_FILE="otp-06-report.json"
USER_ID="${USER_ID:-test_otp_06_user_$TEST_RUN_ID}"

DB_URL="$BRIDGE_DB_URL"
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "OTP-06: Missing Price ACK Row Regression"
echo -e "${YELLOW}========================================${NC}"
echo "Test Run ID: $TEST_RUN_ID"
echo ""

echo -e "${YELLOW}[1/4] Cleaning up previous test data${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID'; DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" \
  2>/dev/null
echo -e "${GREEN}✓ Previous rows removed${NC}"
echo ""

echo -e "${YELLOW}[2/4] Calling /api/v1/verify-purchase with mock Google OTP missing price fields${NC}"
VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
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

if [[ "$HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ verify_purchase failed with HTTP $HTTP_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ verify_purchase returned HTTP 200${NC}"
echo ""

echo -e "${YELLOW}[3/4] Verifying pay.payments keeps ACK identity row${NC}"
PAYMENT_QUERY="SELECT provider_purchase_token, product_id, ack_required, status, acknowledged_at IS NOT NULL FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"
PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$PAYMENT_RESULT" || "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ Expected pay.payments row even when Google returned no price/currency${NC}"
    exit 1
fi

PAYMENT_PURCHASE_TOKEN=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $1}' | tr -d ' ')
PAYMENT_PRODUCT_ID=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $2}' | tr -d ' ')
PAYMENT_ACK_REQUIRED=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $3}' | tr -d ' ')
PAYMENT_STATUS=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $4}' | tr -d ' ')
PAYMENT_ACKED=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $5}' | tr -d ' ')

if [[ "$PAYMENT_PURCHASE_TOKEN" != "$DUMMY_TOKEN" ]]; then
    echo -e "${RED}✗ provider_purchase_token mismatch: $PAYMENT_PURCHASE_TOKEN${NC}"
    exit 1
fi

if [[ "$PAYMENT_PRODUCT_ID" != "$PRODUCT_ID" ]]; then
    echo -e "${RED}✗ product_id mismatch: $PAYMENT_PRODUCT_ID${NC}"
    exit 1
fi

if [[ "$PAYMENT_ACK_REQUIRED" != "t" ]]; then
    echo -e "${RED}✗ ack_required should be true for Google OTP row, got: $PAYMENT_ACK_REQUIRED${NC}"
    exit 1
fi

if [[ "$PAYMENT_STATUS" != "success" ]]; then
    echo -e "${RED}✗ Expected payment status success, got: $PAYMENT_STATUS${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Payment row preserves purchase token, product id, ACK flag, and success status${NC}"
echo ""

echo -e "${YELLOW}[4/4] Writing report${NC}"
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-06",
  "test_name": "Google OTP missing price keeps ACK row",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$DUMMY_TOKEN",
  "http_code": $HTTP_CODE,
  "results": {
    "verify_endpoint_success": true,
    "database_record_found": true,
    "purchase_token_matches": true,
    "product_id_matches": true,
    "ack_required": true,
    "status_is_success": true,
    "acknowledged": $([[ "$PAYMENT_ACKED" == "t" ]] && echo "true" || echo "false")
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ OTP-06 Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
