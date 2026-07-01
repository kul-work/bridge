#!/bin/bash

##############################################################################
# SUB-01: Initial Subscription Onboarding Flow
#
# Purpose: Verify the standard end-to-end subscription onboarding flow:
#          Intent Registration -> Verification -> Persistence.
#
# Usage: ./test-sub-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: A new subscription record is created with status 'active'/'trial'.
#                      A payment record is created in pay.payments with status='success'.
#                      The purchase is marked as 'acknowledged' (acknowledged_at is set).
#                      Ensures the baseline onboarding lifecycle works correctly.
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
TEST_RUN_ID="sub-01-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-sub-01-token-$TEST_RUN_ID"
REPORT_FILE="sub-01-report.json"
USER_ID="${USER_ID:-test_sub_user_01_$TEST_RUN_ID}"
TEST_STATUS="fail"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-01: Bridge Initial Subscription Purchase"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

bridge_log_marker "BEGIN test === SUB-01, run id $TEST_RUN_ID"
trap 'bridge_log_marker "END test === SUB-01, run id $TEST_RUN_ID - $TEST_STATUS"' EXIT

# Step 1: External User ID
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

# Step 3: Call Bridge /api/v1/purchase/register
echo -e "${YELLOW}[1/5] Calling Bridge /api/v1/purchase/register${NC}"

REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-sub-01-setup\"
  }")

echo -e "${GREEN}✓ Purchase registration complete${NC}"
echo ""

# Step 4: Call Bridge /api/v1/verify-purchase
echo -e "${YELLOW}[2/5] Calling Bridge /api/v1/verify-purchase${NC}"

VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

echo -e "${GREEN}✓ Purchase verification complete${NC}"
echo ""

# Step 5: Verify status in DB
echo -e "${YELLOW}[3/5] Verifying 'active' or 'trial' state in Bridge DB${NC}"
export PGPASSWORD="postgres"
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t | tr -d '[:space:]')

if [[ "$STATUS" == "active" ]] || [[ "$STATUS" == "trial" ]]; then
    echo -e "${GREEN}✓ Success: Status is '$STATUS'${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected active/trial${NC}"
    exit 1
fi
echo ""

# Step 6: Verify payment record
echo -e "${YELLOW}[4/5] Verifying payment record and acknowledgement${NC}"
RES_PAY=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, (acknowledged_at IS NOT NULL) as is_ack FROM pay.payments WHERE external_user_id = '$USER_ID' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]')

# Expected: success | t
if [[ "$RES_PAY" == *"success"*"t"* ]]; then
    echo -e "${GREEN}✓ Success: Payment record found and acknowledged${NC}"
else
    echo -e "${RED}✗ Failure: Payment record state mismatch: $RES_PAY${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-01",
  "test_name": "Bridge Initial Subscription Purchase",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$DUMMY_TOKEN",
  "subscription_status": "$STATUS",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "payment_result": "$RES_PAY",
  "database_verified": true,
  "results": {
    "purchase_registered": true,
    "purchase_verified": true,
    "status_is_active_or_trial": true,
    "payment_record_acknowledged": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-01 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
TEST_STATUS="pass"
exit 0
