#!/bin/bash

##############################################################################
# SUB-01: Bridge Initial Subscription Purchase Test (Multi-DB)
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Debug API key
echo "DEBUG: BRIDGE_API_KEY length: ${#BRIDGE_API_KEY}"
echo "DEBUG: BRIDGE_API_KEY first 15 chars: ${BRIDGE_API_KEY:0:15}"
echo "DEBUG: BRIDGE_API_KEY last 15 chars: ${BRIDGE_API_KEY: -15}"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
DUMMY_TOKEN="test-subscription-sub01-12345"
PRODUCT_ID="$PRODUCT_ID_SUB"
USER_ID="clerk_test_$(date +%s)"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-01: Bridge Initial Subscription Purchase Test"
echo "User ID: $USER_ID"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Clean up Bridge DB
echo -e "${YELLOW}[1/6] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "bridge_admin" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null || true
psql -U "bridge_admin" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null || true
echo -e "${GREEN}✓ Previous test data removed${NC}"
echo ""

# Step 2: Call Bridge /api/v1/purchase/register
echo -e "${YELLOW}[2/6] Calling Bridge /api/v1/purchase/register${NC}"
echo "  POST $BRIDGE_API_URL/api/v1/purchase/register"
echo ""

REGISTER_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-registration\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-$(date +%s)\"
  }")

REGISTER_HTTP_CODE=$(echo "$REGISTER_RESPONSE" | tail -n1)
REGISTER_BODY=$(echo "$REGISTER_RESPONSE" | head -n -1)
echo "Response Code: $REGISTER_HTTP_CODE"
echo "Response: $REGISTER_BODY"
echo ""

if [[ "$REGISTER_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ register_purchase failed with HTTP $REGISTER_HTTP_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ register_purchase returned HTTP 200${NC}"
echo ""

# Step 3: Call Bridge /api/v1/verify-purchase
echo -e "${YELLOW}[3/6] Calling Bridge /api/v1/verify-purchase${NC}"

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

VERIFY_HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | head -n -1)
echo "Response Code: $VERIFY_HTTP_CODE"
echo "Response: $VERIFY_BODY"
echo ""

if [[ "$VERIFY_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ verify-purchase failed with HTTP $VERIFY_HTTP_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ verify-purchase returned HTTP 200${NC}"
echo ""

# Step 4: Query Bridge DB to verify subscription storage
echo -e "${YELLOW}[4/6] Querying Bridge DB (pay.subscriptions)${NC}"

DB_QUERY="SELECT external_user_id, subscription_id, status, purchase_token, auto_renewing, current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

DB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "$DB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$DB_RESULT" || "$DB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No subscription record found in Bridge DB${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Subscription record found in Bridge:${NC}"
echo "$DB_RESULT"
echo ""

# Step 5: Verify status is "active" or "trial"
STATUS=$(echo "$DB_RESULT" | awk -F '|' '{print $3}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
if [[ "$STATUS" != "active" ]] && [[ "$STATUS" != "trial" ]]; then
    echo -e "${RED}✗ Expected status 'active' or 'trial', got '$STATUS'${NC}"
    exit 1
fi

# Step 6: Verify payment record
echo -e "${YELLOW}[5/6] Verifying Bridge DB (pay.payments)${NC}"

PAYMENT_QUERY="SELECT amount_cents, status, provider_transaction_id, acknowledged_at FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$PAYMENT_RESULT" || "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in Bridge DB${NC}"
    exit 1
fi

ACKNOWLEDGED_AT=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $4}' | tr -d ' ')

if [[ -z "$ACKNOWLEDGED_AT" ]]; then
    echo -e "${RED}✗ Payment not acknowledged! (acknowledged_at is NULL)${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Payment Record Found and Acknowledged${NC}"
echo ""

echo -e "${YELLOW}[6/6] Test Complete${NC}"
echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ SUB-01 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
