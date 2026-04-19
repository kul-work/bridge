#!/bin/bash

##############################################################################
# SUB-17: Restore After Uninstall/Reinstall
# 
# Purpose: Verify that an active subscription is correctly reported upon 
#          "reinstall" (calling the subscription status API).
#
# Usage: ./test-sub-17.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: GET /api/v1/subscriptions returns 'active' for existing user.
#                      No duplicate entries are created by the lookup/status calls (idempotency).
#                      Ensures users who replace devices or reinstall apps regain access seamlessly.
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
REPORT_FILE="sub-17-report.json"

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
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
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
  }" )

# Verify purchase
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

echo -e "${GREEN}✓ Active subscription established${NC}"
echo ""

# Step 4: Simulate "reinstall" (Call status API)
echo -e "${YELLOW}[2/5] Simulating reinstall: fetching subscription status${NC}"

STATUS_RESPONSE=$(curl -s -X GET \
  "$BRIDGE_API_URL/api/v1/subscriptions?external_user_id=$USER_ID" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

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

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-17",
  "test_name": "Restore After Uninstall/Reinstall",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "idempotency_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-17 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
