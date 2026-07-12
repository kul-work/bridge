#!/bin/bash

##############################################################################
# NET-05: Bridge-to-App Delivery Verification
#
# Purpose: Verify that webhooks are successfully forwarded from Bridge to the
#          downstream application callback URL.
#
# Usage: ./test-net-05.sh
#
# Prerequisites:
#   - Bridge Backend running with MOCK_EXTERNAL_APIS=true
#   - HiHa Backend (or mock) running at the configured callback URL
#   - globals.cfg sourced with required vars
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="net-05-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-net-05-token-$TEST_RUN_ID"
REPORT_FILE="net-05-report.json"
USER_ID="test_net_user_$TEST_RUN_ID"

echo -e "${YELLOW}========================================${NC}"
echo "NET-05: Bridge-to-App Delivery Verification"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Trigger a Webhook Event (Initial Purchase)
echo -e "${YELLOW}[1/3] Triggering purchase event to generate webhook${NC}"

# Register intent
curl -s -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-net-05\"
  }" > /dev/null

# Verify purchase (this triggers the asynchronous webhook forwarding)
VERIFY_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

if [[ "$VERIFY_CODE" != "200" ]]; then
    echo -e "${RED}✗ Error: verify-purchase failed with HTTP $VERIFY_CODE${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Event triggered successfully (Bridge accepted purchase)${NC}"
echo ""

# Step 2: Poll pay.webhook_delivery for success
echo -e "${YELLOW}[2/3] Polling Bridge DB for successful delivery to app${NC}"
echo "      (Allowing up to 10 seconds for async delivery and retries)"

MAX_RETRIES=5
RETRY_INTERVAL=2
SUCCESS=false
DELIVERY_INFO=""

for ((i=1; i<=MAX_RETRIES; i++)); do
    echo "  Attempt $i/$MAX_RETRIES: Checking delivery status..."
    
    DELIVERY_INFO=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT wd.forwarded, wd.last_http_status, wd.last_error 
          FROM pay.webhook_delivery wd
          JOIN pay.webhook_provider wp ON wd.webhook_provider_id = wp.id
          WHERE wp.purchase_token = '$DUMMY_TOKEN' AND wp.app_id = (SELECT id FROM pay.apps WHERE slug = 'hiha' LIMIT 1)
          ORDER BY wd.created_at DESC LIMIT 1;" -t | tr -d '[:space:]')

    if [[ "$DELIVERY_INFO" == "t"* ]]; then
        echo -e "${GREEN}✓ Webhook successfully delivered to app!${NC}"
        SUCCESS=true
        break
    fi
    
    if [[ "$i" -lt "$MAX_RETRIES" ]]; then
        sleep $RETRY_INTERVAL
    fi
done

echo ""

# Step 3: Final Assessment
if [[ "$SUCCESS" == "true" ]]; then
    echo -e "${GREEN}✓ Success: Delivery confirmed ($DELIVERY_INFO)${NC}"
    TEST_STATUS="pass"
else
    echo -e "${RED}✗ Failure: Webhook delivery failed or timed out.${NC}"
    echo -e "${BLUE}Last DB Status: $DELIVERY_INFO${NC}"
    echo ""
    echo -e "${YELLOW}Troubleshooting:${NC}"
    echo "1. Is the HiHa backend running at 'localhost:3000' (or the configured callback URL)?"
    echo "2. Check Bridge logs for 'Failed to forward webhook' errors."
    echo "3. Ensure the 'hiha' app is enabled in pay.apps table."
    TEST_STATUS="fail"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "NET-05",
  "test_name": "Bridge-to-App Delivery Verification",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "results": {
    "purchase_token": "$DUMMY_TOKEN",
    "delivery_confirmed": $SUCCESS,
    "db_info": "$DELIVERY_INFO"
  }
}
EOF

if [[ "$TEST_STATUS" == "fail" ]]; then
    echo -e "${RED}✗ NET-05 Bridge Test FAILED${NC}"
    exit 1
fi

echo -e "${GREEN}✓ NET-05 Bridge Test PASSED${NC}"
exit 0
