#!/bin/bash

##############################################################################
# OTP-01: Successful One-Time Purchase (Webhook)
# 
# Purpose: Verify that a successful Creem checkout.completed webhook is 
#          properly verified and stored in the database with status 'success'.
#
# Usage: ./test-otp-01.sh
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
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
NC='\033[0m'

# Defaults
TIMESTAMP=$(date +%s)
EMAIL="creem_otp_user_$TIMESTAMP@example.com"
USER_ID="test_otp_user_$TIMESTAMP"

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

echo -e "${YELLOW}========================================${NC}"
echo "OTP-01: Successful Purchase (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: User Identity
echo -e "${YELLOW}[1/5] Using External User ID: $USER_ID (Email: $EMAIL)${NC}"
echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Cleanup
echo -e "${YELLOW}[2/5] Cleaning up old data from Bridge DB${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" > /dev/null 2>&1 || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.webhook_log WHERE provider = 'creem' AND provider_webhook_id LIKE 'evt_otp_01_%';" > /dev/null 2>&1 || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Trigger Webhook
echo -e "${YELLOW}[3/5] Sending checkout.completed webhook to Bridge${NC}"
EVENT_ID="evt_otp_01_$(date +%s)"
CHECKOUT_ID="checkout_otp_01_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "checkout.completed",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$CHECKOUT_ID",
    "billing_type": "one_time",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_01"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "completed",
    "last_transaction": {
      "id": "creem_tx_otp_01",
      "amount": 2999
    }
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

# Step 4: Verify DB
echo -e "${YELLOW}[4/5] Verifying Bridge pay.payments table${NC}"
sleep 3 # Allow async processing
QUERY="SELECT status, amount_cents FROM pay.payments WHERE external_user_id = '$USER_ID' AND (product_id = '$PRODUCT_ID_OTP' OR subscription_id = '$PRODUCT_ID_OTP') ORDER BY created_at DESC LIMIT 1;"
PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "$QUERY" -t 2>/dev/null || echo "")

if [[ -z "$PAYMENT_RESULT" ]] || [[ "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ Payment record not found in Bridge DB for query: $QUERY${NC}"
    # Debug: show last 5 payments
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT external_user_id, product_id, status FROM pay.payments ORDER BY created_at DESC LIMIT 5;"
    exit 1
fi

STATUS=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $1}' | tr -d '[:space:]')
AMOUNT=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $2}' | tr -d '[:space:]')

if [[ "$STATUS" == "success" && "$AMOUNT" == "2999" ]]; then
    echo -e "${GREEN}✓ Payment verified: Status=$STATUS, Amount=$AMOUNT${NC}"
else
    echo -e "${RED}✗ Unexpected data: Status=$STATUS, Amount=$AMOUNT (expected success/2999)${NC}"
    exit 1
fi

# Step 5: Report
cat > test-otp-01-report.json <<EOF
{
  "test_id": "OTP-01",
  "status": "pass",
  "user_id": "$USER_ID",
  "event_id": "$EVENT_ID",
  "http_code": $HTTP_CODE,
  "payment_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ OTP-01 PASSED${NC}"

