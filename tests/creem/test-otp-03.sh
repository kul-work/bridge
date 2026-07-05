#!/bin/bash

##############################################################################
# OTP-03: Failed One-Time Purchase
# 
# Purpose: Verify that a Creem payment.failed webhook is properly
#          processed and updates the payment status to 'failed' in the DB.
#
# Usage: ./test-otp-03.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_INGRESS_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
#     * PRODUCT_ID_OTP (for identification)
#   - psql installed and database accessible
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="creem-otp-03-${TIMESTAMP}-$$"
REPORT_FILE="test-otp-03-report.json"
EMAIL="creem_otp_user_${TEST_RUN_ID}@example.com"
USER_ID=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        --user-id)
            USER_ID="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "$USER_ID" ]]; then
    # Generate a unique USER_ID for this run
    USER_ID="creem_user_$TEST_RUN_ID"
fi

echo -e "${YELLOW}========================================${NC}"
echo "OTP-03: Failed Payment (Webhook)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Trigger Webhook
echo -e "${YELLOW}[1/3] Sending payment.failed webhook to Bridge${NC}"
EVENT_ID="evt_fail_03_$TEST_RUN_ID"
CHECKOUT_ID="chk_fail_03_$TEST_RUN_ID"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "payment.failed",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$CHECKOUT_ID",
    "transaction": "$CHECKOUT_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_03"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "product": {
      "currency": "USD"
    },
    "status": "failed",
    "amount": 2999
  }
}
EOF
)

# Use HMAC-SHA256 with Creem Webhook Secret
SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 2: Verify DB
echo -e "${YELLOW}[2/3] Verifying Bridge pay.payments table for 'failed' status${NC}"
sleep 3 # Allow async processing
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE provider_transaction_id = '$CHECKOUT_ID' LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ "$STATUS" == "failed" ]]; then
    echo -e "${GREEN}✓ Payment verified: Status=$STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected status: Status=$STATUS (expected 'failed')${NC}"
    exit 1
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-03",
  "test_name": "Failed One-Time Purchase (Webhook)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "checkout_id": "$CHECKOUT_ID",
  "http_code": $HTTP_CODE,
  "db_status": "$STATUS",
  "results": {
    "webhook_accepted": true,
    "payment_failed_verified": true
  }
}
EOF

echo -e "${GREEN}✓ OTP-03 PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
exit 0
