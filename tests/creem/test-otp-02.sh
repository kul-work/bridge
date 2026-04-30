#!/bin/bash

##############################################################################
# OTP-02: Refund One-Time Purchase
# 
# Purpose: Verify that a Creem checkout.refunded webhook is properly
#          processed and updates the payment status to 'refunded' in the DB.
#
# Usage: ./test-otp-02.sh [--email "user@example.com"] [--user-id "test_user"]
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
TEST_RUN_ID="creem-otp-02-${TIMESTAMP}-$$"
REPORT_FILE="test-otp-02-report.json"
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
echo "OTP-02: Refund Processed (Webhook)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Ensure existing payment exists (from OTP-01)
echo -e "${YELLOW}[1/4] Checking for existing payment to refund${NC}"
CHECKOUT_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ -z "$CHECKOUT_ID" ]]; then
    echo -e "${YELLOW}No payment found. Running OTP-01 first...${NC}"
    ./test-otp-01.sh --user-id "$USER_ID"
    
    CHECKOUT_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "SELECT provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]' || echo "")
fi

if [[ -z "$CHECKOUT_ID" ]]; then
    echo -e "${RED}✗ No successful payment found to refund${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Targeted Checkout: $CHECKOUT_ID${NC}"

# Step 2: Trigger Webhook
echo -e "${YELLOW}[2/4] Sending payment.refunded webhook to Bridge${NC}"
EVENT_ID="evt_refund_02_$TEST_RUN_ID"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "payment.refunded",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "ref_$TEST_RUN_ID",
    "checkout_id": "$CHECKOUT_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_02"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "refunded",
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

# Step 3: Verify DB
echo -e "${YELLOW}[3/4] Verifying Bridge pay.payments table for 'refunded' status${NC}"
sleep 3 # Allow async processing
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE provider_transaction_id = '$CHECKOUT_ID' LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ "$STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Payment verified: Status=$STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected status: Status=$STATUS (expected 'refunded')${NC}"
    exit 1
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-02",
  "test_name": "Refund One-Time Purchase (Webhook)",
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
    "payment_refunded": true
  }
}
EOF

echo -e "${GREEN}✓ OTP-02 PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
exit 0
