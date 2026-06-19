#!/bin/bash

##############################################################################
# SUB-16: Resubscribe Before Expiration (Continuous Access)
# 
# Purpose: Verify the full resubscribe flow before expiration:
#          Establish cancelled sub -> Verify new purchase -> Active status & Token linking.
#
# Usage: ./test-sub-16.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Cancelled record in pay.subscriptions is reactivated (status='active').
#                      purchase_token is updated to the NEW token.
#                      google_linked_purchase_token is populated with the OLD token.
#                      Ensures users can "re-enable" auto-renewal by starting a new logical purchase.
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
TEST_RUN_ID="sub-16-${TIMESTAMP}-$$"
OLD_TOKEN="test-sub-16-old-token-$TEST_RUN_ID"
NEW_TOKEN="test-sub-16-new-token-$TEST_RUN_ID"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-16-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "SUB-16: Resubscribe Before Expiration"
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

# Step 3: Establish a cancelled subscription
echo -e "${YELLOW}[1/5] Establishing a cancelled subscription (still in period)${NC}"

# Seed cancelled subscription
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, current_period_end) 
      VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'cancelled', false, '$OLD_TOKEN', NOW() + INTERVAL '7 days');" > /dev/null

echo -e "${GREEN}✓ Cancelled subscription seeded in DB${NC}"
echo ""

# Step 4: Resubscribe with NEW token
echo -e "${YELLOW}[2/5] Resubscribing with NEW token: $NEW_TOKEN${NC}"

# Pre-register new purchase
REGISTER_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-resub-16\"
  }" )

# Verify purchase with NEW token and mock header for linked token
VERIFY_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "X-Test-Linked-Token: $OLD_TOKEN" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$NEW_TOKEN\",
    \"product_type\": \"subscription\"
  }" )

echo -e "${GREEN}✓ Resubscription request sent${NC}"
echo ""

# Step 5: Final Validation in DB
echo -e "${YELLOW}[3/5] Verifying re-subscription state in Bridge DB${NC}"
export PGPASSWORD="postgres"
RES_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status, purchase_token, google_linked_purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]')

# Expected: active | {NEW_TOKEN} | {OLD_TOKEN}
if [[ "$RES_DATA" == *"active"* ]] && [[ "$RES_DATA" == *"$NEW_TOKEN"* ]] && [[ "$RES_DATA" == *"$OLD_TOKEN"* ]]; then
    echo -e "${GREEN}✓ Success: Status is active, token updated, and linked token preserved${NC}"
else
    echo -e "${RED}✗ Failure: Re-subscription state mismatch: $RES_DATA${NC}"
    exit 1
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-16",
  "test_name": "Resubscribe Before Expiration",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "register_http_code": $REGISTER_HTTP_CODE,
  "verify_http_code": $VERIFY_HTTP_CODE,
  "resubscription_verified": true
}
EOF

echo -e "${GREEN}✓ SUB-16 Bridge Test PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
exit 0
