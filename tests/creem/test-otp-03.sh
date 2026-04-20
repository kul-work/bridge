#!/bin/bash

##############################################################################
# OTP-03: Failed Payment Processing (Webhook)
# 
# Purpose: Verify that a Creem payment.failed webhook is properly 
#          processed, updating the payment record to 'failed'.
#
# Usage: ./test-otp-03.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_TOKEN
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

# Defaults
TIMESTAMP=$(date +%s)
EMAIL="creem_otp_user_$TIMESTAMP@example.com"
USER_ID="test_otp_user_$TIMESTAMP"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
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

echo -e "${YELLOW}========================================${NC}"
echo "OTP-03: Refund Creation (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure existing payment exists (from OTP-01)
echo -e "${YELLOW}[1/4] Checking for existing payment to refund${NC}"
TX_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ -z "$TX_ID" ]]; then
    echo -e "${YELLOW}No payment found. Running OTP-01 first...${NC}"
    ./test-otp-01.sh --user-id "$USER_ID"
    
    TX_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]' || echo "")
fi

if [[ -z "$TX_ID" ]]; then
    echo -e "${RED}✗ No successful payment found to refund${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Targeted Transaction: $TX_ID${NC}"

# Step 2: Send refund.created webhook
echo -e "${YELLOW}[2/4] Sending refund.created webhook${NC}"
EVENT_ID="evt_refund_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "refund.created",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "ref_order_03_$(date +%s)",
    "order_id": "$TX_ID",
    "customer": {
        "email": "$EMAIL",
        "id": "cust_creem_03"
    },
    "checkout": {
      "id": "ch_test_$(date +%s)",
      "metadata": {
        "user_id": "$USER_ID"
      }
    },
    "product_id": "$PRODUCT_ID_OTP",
    "amount": 2999,
    "status": "succeeded"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 3: Verify DB status is 'refunded'
echo -e "${YELLOW}[3/4] Verifying payment status is 'refunded'${NC}"
sleep 2
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND provider_transaction_id = '$TX_ID' LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ "$STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Status verified: $STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected status: $STATUS (Expected: refunded)${NC}"
    exit 1
fi

# Step 4: Report
cat > test-otp-03-report.json <<EOF
{
  "test_id": "OTP-03",
  "status": "pass",
  "user_id": "$USER_ID",
  "event_id": "$EVENT_ID",
  "db_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ OTP-03 PASSED${NC}"
