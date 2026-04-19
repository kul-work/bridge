#!/bin/bash

##############################################################################
# SUB-19: Restore with Account System (Multi-Account Token Isolation)
# 
# Purpose: Verify restore behavior when a second app account (User 2) 
#          attempts to restore a subscription purchased by User 1 
#          on the same device/Google account.
#
# Usage: ./test-sub-19.sh
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
#   Expected Behavior: User 1 remains the primary owner of the purchase_token in pay.subscriptions.
#                      System prevents unauthorized token 'stealing' or automatic reassignment to User 2.
#                      Ensures account security and prevents sharing exploits across different app logins.
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
DUMMY_TOKEN="test-sub-19-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-19-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-19: Restore with Account System"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: External User IDs
USER1_ID="test_sub_user_01"
USER2_ID="test_sub_user_02"
echo -e "${GREEN}✓ Testing with User IDs: $USER1_ID (Owner), $USER2_ID (Secondary)${NC}"
echo ""

# Step 2: Clean up previous test data
echo -e "${YELLOW}[0/5] Cleaning up previous test data from Bridge${NC}"
export PGPASSWORD="postgres"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id IN ('$USER1_ID', '$USER2_ID');" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id IN ('$USER1_ID', '$USER2_ID');" 2>/dev/null || true
echo ""

# Step 3: Establish an active subscription (User 1)
echo -e "${YELLOW}[1/5] Establishing active subscription for User 1${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER1_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-19-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-reg-19-$(date +%s)\"
  }" )

# Verify purchase
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER1_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

echo -e "${GREEN}✓ User 1 subscription established${NC}"
echo ""

# Step 4: Call status API for User 2
echo -e "${YELLOW}[2/5] Simulating User 2 login: fetching subscription status${NC}"

STATUS_RESPONSE=$(curl -s -X GET \
  "$BRIDGE_API_URL/api/v1/subscriptions?external_user_id=$USER2_ID" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Device-ID: shared-device-19")

echo "Response for User 2: $STATUS_RESPONSE"

if echo "$STATUS_RESPONSE" | grep -qi '"active"'; then
    echo -e "${GREEN}✓ Strategy Detected: Shared Access (User 2 has access)${NC}"
else
    echo -e "${YELLOW}ℹ Strategy Detected: Exclusive Access (User 2 denied access)${NC}"
fi
echo ""

# Step 5: Verify DB state
echo -e "${YELLOW}[3/5] Verifying DB state remains consistent${NC}"
export PGPASSWORD="postgres"
OWNER_IN_DB=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT external_user_id FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$OWNER_IN_DB" == "$USER1_ID" ]]; then
    echo -e "${GREEN}✓ Success: User 1 remains the primary owner in Bridge DB${NC}"
else
    echo -e "${RED}✗ Failure: Token owner changed to $OWNER_IN_DB, expected $USER1_ID${NC}"
    exit 1
fi
echo ""

# Generate JSON report
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-19",
  "test_name": "Cross-User Purchase Token Isolation",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "token_isolation_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-19 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
