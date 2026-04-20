#!/bin/bash

##############################################################################
# SUB-12: Manual Payment for Existing Sub (Webhook)
# 
# Purpose: Verify that a Creem payment.success webhook correctly records 
#          a payment for a user with an existing active subscription.
#
# Usage: ./test-sub-12.sh --user-id "test_user"
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
EMAIL="creem_user_$TIMESTAMP@example.com"
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
echo "SUB-12: Subscription Payment Refunded"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure active subscription exists
echo -e "${YELLOW}[1/4] Checking for existing active subscription${NC}"
SUB_EXISTS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT id FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

if [[ -z "$SUB_EXISTS" ]]; then
    echo -e "${YELLOW}No active sub found for $USER_ID. Running SUB-01 first...${NC}"
    ./test-sub-01.sh --user-id "$USER_ID"
fi

echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Trigger Refund Webhook
echo -e "${YELLOW}[2/4] Sending refund.created webhook for subscription payment${NC}"
EVENT_ID="evt_sub_12_$(date +%s)"
REFUND_ID="ref_sub_12_$(date +%s)"
CHARGE_ID="ch_sub_12_$(date +%s)"

# Create a dummy payment to refund in pay.payments
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.payments (external_user_id, provider, provider_transaction_id, subscription_id, amount_cents, currency, status, created_at, app_id) \
      VALUES ('$USER_ID', 'creem', '$CHARGE_ID', '$SUBSCRIPTION_ID', 2999, 'USD', 'success', NOW(), '$BRIDGE_APP_ID') \
      ON CONFLICT (provider, provider_transaction_id) DO NOTHING;" > /dev/null

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "refund.created",
  "object": {
    "id": "$REFUND_ID",
    "order_id": "$CHARGE_ID",
    "amount": 2999,
    "status": "succeeded",
    "checkout": {
      "id": "ch_sub_12_$(date +%s)",
      "metadata": {
        "user_id": "$USER_ID"
      }
    }
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

# Step 3: Verify DB payment status is 'refunded'
echo -e "${YELLOW}[3/4] Verifying payment status is 'refunded'${NC}"
sleep 2
STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND provider_transaction_id = '$CHARGE_ID';" -t | tr -d ' ' || echo "")

if [[ "$STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Status verified: $STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected payment status: $STATUS (Expected: refunded)${NC}"
    exit 1
fi

# Step 4: Report
cat > test-sub-12-report.json <<EOF
{
  "test_id": "SUB-12",
  "status": "pass",
  "user_id": "$USER_ID",
  "payment_id": "$CHARGE_ID",
  "refund_id": "$REFUND_ID"
}
EOF
echo -e "${GREEN}✓ SUB-12 PASSED${NC}"
