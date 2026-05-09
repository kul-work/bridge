#!/bin/bash

##############################################################################
# OTP-RTDN-04: Webhook OTP Canceled (Type 14)
# 
# Purpose: Verify that a oneTimeProductNotification with notificationType: 14
#          (CANCELED) is properly received and updates payment status.
#
# Usage: ./test-otp-rtdn-04.sh [--token "purchase_token"]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
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
TEST_RUN_ID="otp-rtdn-04-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_OTP"
PROVIDER="$PROVIDER"
REPORT_FILE="otp-rtdn-04-report.json"
PURCHASE_TOKEN=""
USER_ID="${USER_ID:-test_otp_user_04_$TEST_RUN_ID}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --token)
            PURCHASE_TOKEN="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "OTP-RTDN-04: Webhook OTP Canceled (Type 14)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Ensure payment record exists
echo -e "${YELLOW}[1/3] Verifying payment record exists${NC}"

# Extract DB password if needed
if [[ -n "${BRIDGE_DB_URL:-}" ]]; then
    export PGPASSWORD="${BRIDGE_DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

if [[ -z "$PURCHASE_TOKEN" ]]; then
    # Always generate a unique user/token for each run to bypass deduplication
    PURCHASE_TOKEN="test-otp-token-04-$TEST_RUN_ID"
    echo -e "${YELLOW}creating setup record for user $USER_ID, token: $PURCHASE_TOKEN...${NC}"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "INSERT INTO pay.payments (app_id, external_user_id, product_id, status, provider_transaction_id, provider, amount_cents, created_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'success', '$PURCHASE_TOKEN', '$PROVIDER', 999, NOW());" > /dev/null
    echo -e "${GREEN}✓ Created test payment record${NC}"
fi

echo "Purchase Token: $PURCHASE_TOKEN"
echo ""

# Step 2: Send webhook
TIMESTAMP_MS=$(date +%s000)
MESSAGE_ID="webhook-otp-cancel-$TEST_RUN_ID"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP_MS",
  "oneTimeProductNotification": {
    "version": "1.0",
    "notificationType": 14,
    "purchaseToken": "$PURCHASE_TOKEN",
    "productId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

echo -e "${YELLOW}[2/3] Sending ONE_TIME_PRODUCT_CANCELED (Type 14) webhook...${NC}"
curl -s -X POST "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    }
  }" > /dev/null

echo -e "${GREEN}✓ Webhook sent${NC}"
sleep 2

# Step 3: Verify in DB
echo -e "${YELLOW}[3/3] Verifying status in DB...${NC}"
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE provider_transaction_id = '$PURCHASE_TOKEN';" -t | tr -d '[:space:]')

if [[ "$STATUS" == "cancelled" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'cancelled'${NC}"
    TEST_STATUS="pass"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected 'cancelled'${NC}"
    TEST_STATUS="fail"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-RTDN-04",
  "test_name": "Webhook OTP Canceled (Type 14)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "webhook_processed": true,
    "final_status": "$STATUS"
  }
}
EOF

echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
