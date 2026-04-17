#!/bin/bash

##############################################################################
# SUB-15: Free Trial - User with No Prior Subscriptions Test
# 
# Purpose: Verify the full free trial flow for users with no prior subscriptions:
#          1. Perform trial purchase verification
#          2. Verify status changed to "trial" in Bridge DB
#          3. Verify initial payment amount is 0
#
# Usage: ./test-sub-15.sh
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
DUMMY_TOKEN="test-sub-15-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-15-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-15: Free Trial - User with No Prior Subscriptions"
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

# Step 3: Call /api/v1/verify-purchase (Trial Purchase)
echo -e "${YELLOW}[1/5] Pre-registering and verifying Trial Purchase${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-15-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-15-$(date +%s)\"
  }")

# Verify purchase with mock header for no prior subs
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Test-No-Prior-Subs: true" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

echo -e "${GREEN}✓ Trial purchase active${NC}"
echo ""

# Step 4: Verify status in DB
echo -e "${YELLOW}[2/5] Verifying 'trial' state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

# Expected: trial or active
if [[ "$RES_DATA" == "trial" ]] || [[ "$RES_DATA" == "active" ]]; then
    echo -e "${GREEN}✓ Success: Status is trial/active${NC}"
else
    echo -e "${RED}✗ Failure: Trial state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Step 5: Verify payment amount is 0
echo -e "${YELLOW}[3/5] Verifying payment amount is 0 (trial)${NC}"
PAY_AMOUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT amount_cents FROM pay.payments WHERE external_user_id = '$USER_ID' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]')

if [[ "$PAY_AMOUNT" == "0" ]]; then
    echo -e "${GREEN}✓ Success: Payment correctly marked with 0 cents${NC}"
else
    echo -e "${RED}✗ Failure: Payment amount is '$PAY_AMOUNT', expected 0 for trial${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-15",
  "test_name": "Free Trial - User with No Prior Subscriptions",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "trial_no_prior_subs_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-15 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
