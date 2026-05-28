#!/bin/bash

##############################################################################
# SUB-12: Subscription Payment Refunded (Webhook)
# 
# Purpose: Verify that a Creem refund.created webhook correctly records 
#          a refund for a subscription and updates the payment status.
#
# Usage: ./test-sub-12.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_INGRESS_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
#     * PRODUCT_ID_SUB (for identification)
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
TEST_RUN_ID="creem-sub-12-${TIMESTAMP}-$$"
REPORT_FILE="test-sub-12-report.json"
EMAIL="creem_user_${TEST_RUN_ID}@example.com"
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
echo "SUB-12: Subscription Payment Refunded"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

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
EVENT_ID="evt_sub_12_$TEST_RUN_ID"
REFUND_ID="ref_sub_12_$TEST_RUN_ID"
CHARGE_ID="ch_sub_12_$TEST_RUN_ID"

# Create a dummy payment to refund in pay.payments
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "INSERT INTO pay.payments (external_user_id, provider, provider_transaction_id, subscription_id, amount_cents, currency, status, created_at, app_id) \
          VALUES ('$USER_ID', 'creem', '$CHARGE_ID', '$SUBSCRIPTION_ID', 2999, 'USD', 'success', NOW(), '$BRIDGE_APP_ID') \
          ON CONFLICT (app_id, provider, provider_transaction_id) DO NOTHING;" > /dev/null

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "refund.created",
  "object": {
    "id": "$REFUND_ID",
    "order_id": "$CHARGE_ID",
    "transaction": "$CHARGE_ID",
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

# Step 4: Verify subscription access is revoked
echo -e "${YELLOW}[4/4] Verifying subscription status is 'revoked'${NC}"
SUB_STATE=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -At -F '|' -c "SELECT status, COALESCE(revocation_reason, '') FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" \
  | tr -d '\r' | head -n 1 || echo "")

if [[ "$SUB_STATE" == "revoked|REFUND" ]]; then
    echo -e "${GREEN}✓ Subscription revoked due to refund${NC}"
else
    echo -e "${RED}✗ Unexpected subscription state: $SUB_STATE (Expected: revoked|REFUND)${NC}"
    exit 1
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "SUB-12",
  "test_name": "Subscription Payment Refunded (Webhook)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "payment_id": "$CHARGE_ID",
  "refund_id": "$REFUND_ID",
  "results": {
    "webhook_accepted": true,
    "payment_refunded": true,
    "subscription_revoked": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-12 PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
exit 0
