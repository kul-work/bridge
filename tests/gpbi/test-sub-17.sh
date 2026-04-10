#!/bin/bash

##############################################################################
# SUB-17: Restore After Uninstall/Reinstall Test
# 
# Purpose: Verify that an active subscription is correctly reported upon 
#          "reinstall" (calling the subscription status API).
#          1. Establish an active subscription
#          2. Call the subscription list API
#          3. Verify status returned is "active"
#          4. Verify no duplicate rows created
#
# Usage: ./test-sub-17.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * PRODUCT_ID_SUB, PROVIDER, PACKAGE_NAME
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
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
TIMESTAMP=$(date +%s)
DUMMY_TOKEN="test-sub-17-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-17: Restore After Uninstall/Reinstall"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User ID
USER_ID="test_sub_user_01"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo ""

# Step 3: Establish an active subscription
echo -e "${YELLOW}[1/5] Establishing an active subscription for restore test${NC}"

# Pre-register purchase
curl -s -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-17-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-17-$(date +%s)\"
  }" > /dev/null

# Verify purchase
curl -s -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

echo -e "${GREEN}✓ Active subscription established${NC}"
echo ""

# Step 4: Simulate "reinstall" (Call status API)
echo -e "${YELLOW}[2/5] Simulating reinstall: fetching subscription status${NC}"

STATUS_RESPONSE=$(curl -s -X GET \
  "$BRIDGE_API_URL/api/v1/subscriptions" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-external-user-id: $USER_ID")

# Verify response
if echo "$STATUS_RESPONSE" | grep -qi '"active"'; then
    echo -e "${GREEN}✓ Success: Status API returned 'active'${NC}"
else
    echo -e "${RED}✗ Failure: Status API did not return 'active' status. Response: $STATUS_RESPONSE${NC}"
    exit 1
fi
echo ""

# Step 5: Verify no duplicates in DB
echo -e "${YELLOW}[3/5] Verifying no duplicate subscription rows created${NC}"
export PGPASSWORD="postgres"
SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]')

if [[ "$SUB_COUNT" == "1" ]]; then
    echo -e "${GREEN}✓ Success: Exactly one subscription record found${NC}"
else
    echo -e "${RED}✗ Failure: Found $SUB_COUNT subscription records, expected 1${NC}"
    exit 1
fi
echo ""

echo -e "${GREEN}✓ SUB-17 Bridge Test PASSED${NC}"
exit 0
