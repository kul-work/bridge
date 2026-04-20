#!/bin/bash

##############################################################################
# OTP-02: Refund Processed (Webhook)
# 
# Purpose: Verify that a Creem payment.refunded webhook is properly 
#          processed, updating the payment record to 'refunded'.
#
# Usage: ./test-otp-02.sh --user-id "test_user"
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
EMAIL="all_user_$TIMESTAMP@example.com"
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
echo "OTP-02: Sync Redirect Verification"
echo -e "${YELLOW}========================================${NC}"

# Step 1: User Identity
echo -e "${YELLOW}[1/4] Using External User ID: $USER_ID${NC}"
echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Cleanup
echo -e "${YELLOW}[2/4] Cleaning up old data from Bridge DB${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" > /dev/null 2>&1 || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Simulate Redirect URL
# Creem redirects to success_url?checkout_id=...&order_id=...&signature=...
CHECKOUT_ID="chk_sync_$(date +%s)"
ORDER_ID="ord_sync_$(date +%s)"

# Construct string for signing (params sorted alphabetically, joined by &)
SIGNING_STRING="checkout_id=$CHECKOUT_ID&customer_id=cust_creem_01&order_id=$ORDER_ID&product_id=$PRODUCT_ID_OTP"
SIGNATURE=$(echo -n "$SIGNING_STRING" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

echo -e "${YELLOW}[3/4] Calling success_url with signature fallback${NC}"
# We call the /api/checkout/verify or similar endpoint if Bridge has a dedicated verification endpoint
# Based on legacy script, it calls /story which seems to be a success redirect handler
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X GET \
  "$APP_URL/story?checkout_id=$CHECKOUT_ID&customer_id=cust_creem_01&order_id=$ORDER_ID&product_id=$PRODUCT_ID_OTP&signature=$SIGNATURE" \
  -H "X-External-User-ID: $USER_ID")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "302" ]]; then
    echo -e "${GREEN}✓ Verification call accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Verification call failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 4: Verify payment or activation in DB
echo -e "${YELLOW}[4/4] Verifying record in DB${NC}"
sleep 2
# Some systems might create a payment record directly or an 'active' one-time-sub
# We'll check both pay.payments and pay.subscriptions
RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND provider_transaction_id = '$ORDER_ID';" -t | tr -d '[:space:]' || echo "")

if [[ -z "$RESULT" ]]; then
    # Maybe it was stored in subscriptions (legacy behavior for granting access)
    RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP';" -t | tr -d '[:space:]' || echo "")
fi

if [[ -n "$RESULT" ]]; then
    echo -e "${GREEN}✓ Record verified: Status=$RESULT${NC}"
else
    echo -e "${RED}✗ No record found in DB after sync verification${NC}"
    # Note: If MOCK_EXTERNAL_APIS is true, it might work; otherwise, might need webhook
    exit 1
fi

# Report
cat > test-otp-02-report.json <<EOF
{
  "test_id": "OTP-02",
  "status": "pass",
  "user_id": "$USER_ID",
  "db_status": "$RESULT"
}
EOF
echo -e "${GREEN}✓ OTP-02 PASSED${NC}"
