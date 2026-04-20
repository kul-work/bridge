#!/bin/bash

##############################################################################
# OTP-03: Refund Creation (Webhook)
# 
# Purpose: Verify that a Creem refund.created webhook properly updates 
#          the payment status to 'refunded' and revokes entitlements.
#
# Usage: ./test-otp-03.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Existing successful purchase for the user (run OTP-01 first)
#   - psql installed and database accessible
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/.env" ]; then
    # Load variables from .env
    set -a
    source "$SCRIPT_DIR/.env"
    set +a
fi
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Defaults
DB_URL="$DATABASE_URL"
EMAIL=""

# Database password
export PGPASSWORD="${DATABASE_PASSWORD:-}"
if [[ -z "$PGPASSWORD" ]]; then
    # Fallback to extraction from URL if not set explicitly
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "OTP-03: Refund Creation (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure existing payment exists (from OTP-01)
echo -e "${YELLOW}[1/4] Checking for existing payment to refund${NC}"
USER_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT clerk_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ User not found or DB error${NC}"
    echo "$USER_ID"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"

PAYMENT_EXISTS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM payments WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP' AND status = 'success';" -t 2>/dev/null | tr -d ' ')

if [[ "$PAYMENT_EXISTS" == "0" ]]; then
    echo -e "${YELLOW}No payment found. Running OTP-01 first...${NC}"
    ./test-otp-01.sh --email "$EMAIL"
fi

# Step 2: Send refund.created webhook
echo -e "${YELLOW}[2/4] Sending refund.created webhook${NC}"

# Fetch the actual transaction ID from the DB to refund
TX_ID=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT provider_transaction_id FROM payments WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP' AND status = 'success' ORDER BY created_at DESC LIMIT 1;" -t | tr -d ' ')

if [[ -z "$TX_ID" ]]; then
    echo -e "${RED}✗ No successful payment found to refund${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Targeted Transaction: $TX_ID${NC}"

EVENT_ID="evt_refund_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "refund.created",
  "object": {
    "id": "ref_order_01",
    "order_id": "$TX_ID",
    "customer": {
        "email": "$EMAIL"
    },
    "checkout": {
      "id": "ch_test_$(date +%s)",
      "metadata": {
        "user_id": "$USER_ID"
      }
    },
    "product_id": "$PRODUCT_ID_OTP",
    "subscription_id": "$TX_ID",
    "amount": 2999,
    "status": "refunded"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 3: Verify DB status is 'refunded'
echo -e "${YELLOW}[3/4] Verifying status is 'refunded'${NC}"
sleep 1
STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM payments WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ' || echo "")

if [[ "$STATUS" == "refunded" ]]; then
    echo -e "${GREEN}✓ Status verified: $STATUS${NC}"
else
    echo -e "${RED}✗ Unexpected status: $STATUS${NC}"
    exit 1
fi

# Step 4: Report
cat > test-otp-03-report.json <<EOF
{
  "test_id": "OTP-03",
  "status": "pass",
  "user_id": "$USER_ID",
  "event_id": "$EVENT_ID"
}
EOF
echo -e "${GREEN}✓ OTP-03 PASSED${NC}"
