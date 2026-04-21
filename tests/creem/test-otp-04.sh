#!/bin/bash

##############################################################################
# OTP-04: Partially Refunded One-Time Purchase
# 
# Purpose: Verify that a Creem payment.partially_refunded webhook is properly
#          processed and updates the payment status to 'partially_refunded' in the DB.
#
# Usage: ./test-otp-04.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
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

# Defaults
TIMESTAMP=$(date +%s)
EMAIL="creem_otp_user_$TIMESTAMP@example.com"
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
    # Generate a stable-ish USER_ID from email if not provided
    USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"
fi

echo -e "${YELLOW}========================================${NC}"
echo "OTP-04: Partially Refunded (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure existing payment exists (from OTP-01)
echo -e "${YELLOW}[1/4] Checking for existing payment to partially refund${NC}"
CHECKOUT_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ -z "$CHECKOUT_ID" ]]; then
    echo -e "${YELLOW}No payment found. Running OTP-01 first...${NC}"
    ./test-otp-01.sh --user-id "$USER_ID"
    
    CHECKOUT_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "SELECT provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d '[:space:]' || echo "")
fi

if [[ -z "$CHECKOUT_ID" ]]; then
    echo -e "${RED}✗ No successful payment found to partially refund${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Targeted Checkout: $CHECKOUT_ID${NC}"

# Step 2: Trigger Webhook
echo -e "${YELLOW}[2/4] Sending payment.partially_refunded webhook to Bridge${NC}"
EVENT_ID="evt_part_refund_04_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "payment.partially_refunded",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "pref_$(date +%s)",
    "checkout_id": "$CHECKOUT_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_04"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "partially_refunded",
    "amount": 1499
  }
}
EOF
)

# Use HMAC-SHA256 with Creem Webhook Secret
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

# Step 3: Verify DB
echo -e "${YELLOW}[3/4] Verifying Bridge pay.payments table for 'partially_refunded' status${NC}"
sleep 3 # Allow async processing
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE provider_transaction_id = '$CHECKOUT_ID' LIMIT 1;" -t | tr -d '[:space:]' || echo "")

if [[ "$STATUS" == "partially_refunded" ]]; then
    echo -e "${GREEN}✓ Payment verified: Status=$STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected status: Status=$STATUS (expected 'partially_refunded')${NC}"
    exit 1
fi

# Step 4: Report
cat > test-otp-04-report.json <<EOF
{
  "test_id": "OTP-04",
  "status": "pass",
  "user_id": "$USER_ID",
  "checkout_id": "$CHECKOUT_ID",
  "http_code": $HTTP_CODE,
  "db_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ OTP-04 PASSED${NC}"
