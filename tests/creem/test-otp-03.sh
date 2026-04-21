#!/bin/bash

##############################################################################
# OTP-03: Failed Payment (Webhook)
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
echo "OTP-03: Failed Payment (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Trigger Webhook
echo -e "${YELLOW}[1/3] Sending payment.failed webhook to Bridge${NC}"
EVENT_ID="evt_fail_03_$(date +%s)"
CHECKOUT_ID="chk_fail_03_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "payment.failed",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$CHECKOUT_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_03"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "failed",
    "amount": 2999
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

# Step 3: Report
cat > test-otp-03-report.json <<EOF
{
  "test_id": "OTP-03",
  "status": "pass",
  "user_id": "$USER_ID",
  "checkout_id": "$CHECKOUT_ID",
  "http_code": $HTTP_CODE,
  "db_status": "$STATUS"
}
EOF
echo -e "${GREEN}✓ OTP-03 PASSED${NC}"
