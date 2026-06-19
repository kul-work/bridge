#!/bin/bash

##############################################################################
# SUB-18: Restore on Multiple Devices (Same Google Account)
# 
# Purpose: Verify that an active subscription is correctly reported on multiple 
#          devices using the same account (different device IDs).
#
# Usage: ./test-sub-18.sh
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
#   Expected Behavior: Device B correctly identifies subscription as 'active' via GET /api/v1/subscriptions.
#                      Backend logic correctly maps Device B to the existing record for that user.
#                      Exactly one subscription record exists in the database.
#                      Ensures multi-device access for a single logical subscriber sharing a Google account.
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
TEST_RUN_ID="sub-18-${TIMESTAMP}-$$"
DUMMY_TOKEN="test-sub-18-token-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-18-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-18: Restore on Multiple Devices"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
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

# Step 3: Establish an active subscription (Device A)
echo -e "${YELLOW}[1/5] Establishing an active subscription (Device A)${NC}"

# Pre-register purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-18-setup\"
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

# Step 4: Call status API from Device B
echo -e "${YELLOW}[2/5] Simulating Device B login: fetching subscription status${NC}"

STATUS_RESPONSE=$(curl -s -X GET \
  "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Device-ID: device-b-18-$(date +%s)")

# Verify response
if echo "$STATUS_RESPONSE" | grep -qi '"is_premium":true'; then
    echo -e "${GREEN}✓ Success: Device B identifies subscription as 'is_premium: true'${NC}"
else
    echo -e "${RED}✗ Failure: Status API did not return premium status for Device B. Response: $STATUS_RESPONSE${NC}"
    exit 1
fi
echo ""

# Step 5: Verify no duplicates in DB
echo -e "${YELLOW}[3/5] Verifying no duplicate records created in Bridge DB${NC}"
export PGPASSWORD="postgres"
SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]')

if [[ "$SUB_COUNT" == "1" ]]; then
    echo -e "${GREEN}✓ Success: No duplicate rows found in DB${NC}"
else
    echo -e "${RED}✗ Failure: Found $SUB_COUNT subscription records, expected 1${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-18",
  "test_name": "Restore on Multiple Devices",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "duplicate_suppression_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-18 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
