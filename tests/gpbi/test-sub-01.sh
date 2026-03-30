#!/bin/bash

##############################################################################
# SUB-01: Bridge Initial Subscription Purchase Test
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
DUMMY_TOKEN="test-subscription-sub01-12345"
PRODUCT_ID="$PRODUCT_ID_SUB"

# Defaults
EMAIL="test-user@example.com"
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-01: Bridge Initial Subscription Purchase Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID (just use a test string for Bridge)
USER_ID="clerk_test_$(date +%s)"
echo -e "${GREEN}✓ External User ID (test): $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing entries from previous tests
echo -e "${YELLOW}[2/6] Cleaning up previous test data${NC}"

CLEANUP_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$CLEANUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous subscription record removed${NC}"
echo ""

# Step 3: Call /api/v1/purchases/register (pre-registration)
echo -e "${YELLOW}[3/6] Calling /api/v1/purchases/register (pre-registration)${NC}"

echo "  POST $APP_URL/api/v1/purchases/register"
echo ""

REGISTER_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/purchases/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"product_id\": \"$PRODUCT_ID\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-$(date +%s)\"
  }")

REGISTER_HTTP_CODE=$(echo "$REGISTER_RESPONSE" | tail -n1)
echo "Response Code: $REGISTER_HTTP_CODE"

if [[ "$REGISTER_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ registrations failed with HTTP $REGISTER_HTTP_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ register_purchase returned HTTP 200${NC}"
echo ""

# Step 4: Call /api/v1/verify-purchase endpoint
echo -e "${YELLOW}[4/6] Calling /api/v1/verify-purchase${NC}"

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | head -n -1)
echo "Response Code: $HTTP_CODE"
echo "Response: $VERIFY_BODY"
echo ""

if [[ "$HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ verify-purchase failed with HTTP $HTTP_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ verify-purchase returned HTTP 200${NC}"
echo ""

# Step 5: Query database to verify storage
echo -e "${YELLOW}[5/6] Querying database to verify storage (pay.subscriptions)${NC}"

DB_QUERY="SELECT external_user_id, subscription_id, status, purchase_token, auto_renewing, current_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

DB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$DB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$DB_RESULT" || "$DB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No subscription record found in database${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Subscription record found:${NC}"
echo "$DB_RESULT"
echo ""

# Step 6: Verify status is "active" or "trial"
STATUS=$(echo "$DB_RESULT" | awk -F '|' '{print $3}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
if [[ "$STATUS" != "active" ]] && [[ "$STATUS" != "trial" ]]; then
    echo -e "${RED}✗ Expected status 'active' or 'trial', got '$STATUS'${NC}"
    exit 1
fi

# Verify payment record in pay.payments table
echo -e "${YELLOW}[6/6] Verifying pay.payments table record${NC}"

PAYMENT_QUERY="SELECT amount_cents, status, provider_transaction_id, acknowledged_at FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

PAYMENT_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$PAYMENT_RESULT" || "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in pay.payments table${NC}"
    exit 1
fi

ACKNOWLEDGED_AT=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $4}' | tr -d ' ')

if [[ -z "$ACKNOWLEDGED_AT" ]]; then
    echo -e "${RED}✗ Payment not acknowledged! (acknowledged_at is NULL)${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Payment Record Found and Acknowledged${NC}"
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ SUB-01 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
